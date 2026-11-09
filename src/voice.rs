//! 语音输入（v0.4.0 开发中）：设备录音 → 本机转写 → 投递到 WorkBuddy 输入框。
//!
//! 设备侧的录音由 `zectrix_sdk::Event` 直接给 16 kHz 单声道 PCM（长按确认键，
//! 松开结束）；这里只负责主机侧两件事：
//!
//! 1. **转写**：SDK 不提供语音识别，必须另做。这里调一个可配置的外部命令
//!    （默认 whisper CLI），`{wav}` 是占位符，识别结果取 stdout——换引擎只改命令。
//! 2. **投递**：WorkBuddy 没有对外的「发消息」接口（`workbuddy://chat/<id>` 只能
//!    切会话，没有预填/自动发送参数），只能走界面：找到输入框的 Edit 节点 →
//!    `SetFocus` → 剪贴板 + Ctrl+V（可选回车发送）。
//!
//! 实测要点（2026-11-09，WorkBuddy 5.9.3）：
//! - 输入框是树里唯一的 Edit（ControlType=50004），可聚焦，但**名字为空**；
//! - `ElementFromPoint` 会被 Chromium 的 `Input Occlusion Window` 挡住，
//!   所以不要靠坐标点击定位，直接 `SetFocus` 到节点更稳；
//! - 窗口的无障碍树会「退化」：实测出现过整棵树只剩 9 个节点（只剩一个 D3D
//!   合成面 + 标题栏），此时定位必然失败——必须如实报错，不能瞎点；
//! - 该窗口类名为 `Chrome_WidgetWin_1`，标题是 `WorkBuddy`；用深链
//!   `workbuddy://chat/<会话 id>` 可以把目标会话切到前台（切会话用得上）。

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// 一笔录音落盘后的结果。
#[derive(Debug, Clone)]
pub struct Clip {
    pub path: PathBuf,
    pub seconds: f32,
    pub peak: u16,
}

/// 一次语音投递的候选会话。`session` 用来拼深链，`title` 只上屏。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub session: String,
    pub title: String,
}

/// 「识别完了，等你决定投给谁」的草稿。
///
/// 只在**设备被占用**（有任务在跑）时才会出现：录音由设备发起，松手就把音频
/// 交给主机，主机转成文字后先给用户看一眼，确认才粘贴——识别错字还有机会
/// 拦下来，不会一松手就往对话里塞一段错话。
#[derive(Clone, Debug)]
pub struct Draft {
    pub text: String,
    /// 录音时长（秒），用来给「这段听清了没有」一个可核对的依据。
    pub seconds: f32,
    pub targets: Vec<Target>,
    /// 当前选中的目标下标。
    pub index: usize,
    /// 到点自动丢弃：设备不在手边时不至于一直占着屏幕。
    pub expires_at: i64,
    /// 上次投递失败的原因。留着文本让人可以直接再按一次，不用重新说一遍。
    pub hint: Option<String>,
}

impl Draft {
    pub fn new(text: String, seconds: f32, targets: Vec<Target>, now: i64) -> Self {
        Self {
            text,
            seconds,
            targets,
            index: 0,
            expires_at: now + PREVIEW_SECS,
            hint: None,
        }
    }
    pub fn target(&self) -> Option<&Target> {
        self.targets.get(self.index)
    }
    /// 上下键换目标。只有一个会话时保持在原位，不绕圈。
    pub fn cycle(&mut self, step: i32) {
        let n = self.targets.len();
        if n < 2 {
            return;
        }
        let next = self.index as i64 + i64::from(step);
        self.index = next.rem_euclid(n as i64) as usize;
    }
}

/// 识别结果在屏幕上停留多久（秒）。
pub const PREVIEW_SECS: i64 = 20;

/// 把 SDK 给的 PCM 写成 16 kHz 单声道 WAV，返回路径、时长与峰值。
pub fn save_wav(dir: &Path, pcm: &[i16]) -> Result<Clip> {
    std::fs::create_dir_all(dir).with_context(|| format!("建目录失败：{}", dir.display()))?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let path = dir.join(format!("voice-{stamp}.wav"));
    let peak = pcm.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
    let data_len = (pcm.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend(b"RIFF");
    b.extend((36 + data_len).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(16000u32.to_le_bytes());
    b.extend(32000u32.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend(data_len.to_le_bytes());
    for s in pcm {
        b.extend(s.to_le_bytes());
    }
    std::fs::write(&path, &b)?;
    Ok(Clip {
        path,
        seconds: pcm.len() as f32 / 16000.0,
        peak,
    })
}

/// 读一遍 WAV 头部，把采样率/位宽/声道翻译成人话（转写前的自检用）。
pub fn describe_wav(path: &Path) -> Result<String> {
    let raw = std::fs::read(path)?;
    if raw.len() < 44 || &raw[..4] != b"RIFF" || &raw[8..12] != b"WAVE" {
        anyhow::bail!("不是 WAV 文件");
    }
    let channels = u16::from_le_bytes([raw[22], raw[23]]);
    let rate = u32::from_le_bytes([raw[24], raw[25], raw[26], raw[27]]);
    let bits = u16::from_le_bytes([raw[34], raw[35]]);
    let secs = (raw.len() - 44) as f32 / (rate as f32 * channels as f32 * (bits as f32 / 8.0));
    Ok(format!("{rate} Hz / {channels} 声道 / {bits} bit / {secs:.2} 秒"))
}

/// 默认模型。small 在 4 GB 显存的 3050 Ti 上跑 10 秒语音约 0.6 秒，
/// 中文准确率够日常指令；想更准可以换成 medium（慢 2～3 倍）。
pub const DEFAULT_MODEL: &str = "small";

/// 常驻转写进程：加载一次模型，之后每段音频只花推理时间。
///
/// 和 `transcribe`（一次性命令）的区别只在等待时间：一次性命令每次要重新
/// import torch 并载模型，实测 6～8 秒；常驻进程 0.5～2 秒。语音是「松手就要
/// 看见字」的交互，这个差别决定了功能能不能用。
pub struct Asr {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    lines: std::sync::mpsc::Receiver<String>,
    pub model: String,
}

impl Asr {
    /// 起一个常驻转写进程，等到它把模型加载完（`ready`）才返回。
    pub fn spawn() -> Result<Self> {
        Self::spawn_with(&python_exe(), &worker_script()?, DEFAULT_MODEL)
    }

    pub fn spawn_with(python: &Path, script: &Path, model: &str) -> Result<Self> {
        use std::io::{BufRead, BufReader};
        let mut cmd = std::process::Command::new(python);
        cmd.arg(script)
            .arg(model)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // 模型加载进度、torch 警告都走 stderr；吞掉，不占日志也不弹窗。
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // CREATE_NO_WINDOW：守护进程是无窗口的，别在用户桌面闪一个黑框。
            cmd.creation_flags(0x0800_0000);
        }
        let mut child = cmd
            .spawn()
            .with_context(|| format!("启动转写进程失败：{}", python.display()))?;
        let stdin = child.stdin.take().context("转写进程没有 stdin")?;
        let stdout = child.stdout.take().context("转写进程没有 stdout")?;
        let (tx, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let asr = Self {            child,
            stdin,
            lines,
            model: model.to_string(),
        };
        // 首次要载模型（small 约 1～3 秒，含 import torch 更久），给足时间。
        match asr.next_json(std::time::Duration::from_secs(120)) {
            Ok(v) if v["ready"].as_bool() == Some(true) => Ok(asr),
            Ok(v) => anyhow::bail!(
                "{}",
                v["error"].as_str().unwrap_or("转写进程启动异常")
            ),
            Err(e) => anyhow::bail!("转写进程没就绪：{e}"),
        }
    }

    fn next_json(&self, wait: std::time::Duration) -> Result<serde_json::Value> {
        let line = self
            .lines
            .recv_timeout(wait)
            .map_err(|_| anyhow::anyhow!("等了 {} 秒没有响应", wait.as_secs()))?;
        serde_json::from_str(&line).with_context(|| format!("转写进程输出不是 JSON：{line}"))
    }

    /// 转写一段 WAV。阻塞调用，调用方要放到 `spawn_blocking` 里。
    pub fn transcribe(&mut self, wav: &Path) -> Result<String> {
        use std::io::Write;
        writeln!(self.stdin, "{}", wav.display()).context("把音频路径写给转写进程失败")?;
        self.stdin.flush().context("转写请求没有送达")?;
        let v = self.next_json(std::time::Duration::from_secs(180))?;
        if let Some(err) = v["error"].as_str() {
            anyhow::bail!("{err}");
        }
        Ok(v["text"].as_str().unwrap_or("").trim().to_string())
    }
}

impl Drop for Asr {
    fn drop(&mut self) {
        use std::io::Write;
        let _ = writeln!(self.stdin, "__quit__");
        let _ = self.stdin.flush();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Python 解释器：优先显式环境变量，其次 python.org 的默认安装位置。
///
/// 不能用 PATH 里的 `python` 兜底就完事——WorkBuddy 自带的 3.13 也在 PATH 里，
/// 但 torch/whisper 装在 3.12 的 user site 下，走错解释器会报「没有 whisper」。
pub fn python_exe() -> PathBuf {
    if let Ok(p) = std::env::var("WORKBUDDY_PET_PYTHON") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let mut candidates = Vec::new();
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        candidates.push(
            PathBuf::from(&local).join(r"Programs\Python\Python312\python.exe"),
        );
    }
    candidates.push(PathBuf::from(r"C:\Python312\python.exe"));
    candidates.push(PathBuf::from(r"C:\Program Files\Python312\python.exe"));
    candidates
        .into_iter()
        .find(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("python"))
}

/// `asr_worker.py` 的位置：显式路径 → 程序同目录 → 仓库 scripts/ → 释放内置副本。
///
/// 最后一档是为了「只有一个 exe」的分发形态：脚本编进二进制，第一次用到时
/// 写到数据目录。这样下载单文件的人不用再找配套文件，而想改脚本的人把它放到
/// exe 同目录就能覆盖。
pub fn worker_script() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("WORKBUDDY_PET_ASR_SCRIPT") {
        if !p.is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // 放在 exe 旁边的那份优先：想临时改脚本不用重新编译。
            candidates.push(dir.join("asr_worker.py"));
        }
    }
    if let Some(path) = candidates.into_iter().find(|p| p.exists()) {
        return Ok(path);
    }
    // 其次是编进二进制的那份（升级后自动同步成新版）。
    if let Some(path) = embedded_script() {
        return Ok(path);
    }
    // 最后才是仓库里的源文件：只用于 `cargo run` 这种没写数据目录的场景。
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(r"scripts\asr_worker.py");
    if dev.exists() {
        return Ok(dev);
    }
    anyhow::bail!("找不到也放不出 asr_worker.py（数据目录不可写？）")
}

/// 编进二进制的那份脚本。
const WORKER_SOURCE: &str = include_str!("../scripts/asr_worker.py");

/// 把内置脚本写到数据目录，内容不同才写（升级后自动换成新版）。
fn embedded_script() -> Option<PathBuf> {
    let base = std::env::var("LOCALAPPDATA").ok()?;
    let path = PathBuf::from(base)
        .join("zectrix-workbuddy-pet")
        .join("asr_worker.py");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }
    if std::fs::read_to_string(&path).ok().as_deref() != Some(WORKER_SOURCE) {
        std::fs::write(&path, WORKER_SOURCE).ok()?;
    }
    Some(path)
}

/// 把文本送进 WorkBuddy 输入框；`send` 为真时再按一次回车。
pub fn deliver(text: &str, send: bool) -> Result<()> {
    if text.trim().is_empty() {
        anyhow::bail!("没有可发送的文本");
    }
    imp::deliver(text, send)
}

/// 诊断：窗口状态 + 输入框定位结果。
pub fn probe() -> Result<String> {
    imp::probe()
}

/// 诊断：列出所有 Chromium 顶层窗口。
pub fn list_windows() -> Vec<String> {
    imp::list_windows()
}

/// 把目标会话切到前台（WorkBuddy 自己处理深链，重复打开同一会话是幂等的）。
pub fn open_session(session: &str) -> Result<()> {
    imp::open_session(session)
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::core::VARIANT;
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    };
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationElement, TreeScope_Descendants,
        UIA_ControlTypePropertyId, UIA_EditControlTypeId,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_CONTROL,
        VK_RETURN, VK_V,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowRect, GetWindowTextW, IsIconic, IsWindowVisible,
        SetForegroundWindow, ShowWindow, SW_RESTORE,
    };

    const CF_UNICODETEXT: u32 = 13;

    pub fn deliver(text: &str, send: bool) -> Result<()> {
        // 先把窗口提到前台再找控件：渲染进程只在窗口被激活时才重建无障碍树，
        // 顺序反了会一直看到退化树（实测 0 个 Edit）。
        let _ = focus_window();
        std::thread::sleep(std::time::Duration::from_millis(400));
        let (_automation, element) = composer()?;
        let mut focused = false;
        for _ in 0..5 {
            if unsafe { element.SetFocus() }.is_ok() {
                focused = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(120));
        }
        if !focused {
            anyhow::bail!("输入框聚焦失败（窗口可能不在前台），请手动点一下输入框再试");
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
        set_clipboard(text).context("写剪贴板失败（可能有别的程序正占着剪贴板）")?;
        key_combo(&[VK_CONTROL], VK_V).context("发送 Ctrl+V 失败")?;
        if send {
            std::thread::sleep(std::time::Duration::from_millis(250));
            key_combo(&[], VK_RETURN).context("发送回车失败")?;
        }
        Ok(())
    }

    pub fn probe() -> Result<String> {
        let mut out = String::new();
        for line in list_windows() {
            out.push_str(&line);
            out.push('\n');
        }
        match composer() {
            Ok(_) => out.push_str("输入框：已定位（Edit 节点，可 SetFocus）"),
            Err(e) => out.push_str(&format!("输入框：定位失败——{e}")),
        }
        Ok(out)
    }

    pub fn open_session(session: &str) -> Result<()> {
        // 必须清掉 ELECTRON_RUN_AS_NODE：从 Electron 派生的 shell 里带这个变量，
        // 会把 URL 当脚本路径交给 node，直接报 MODULE_NOT_FOUND。
        let url = format!("workbuddy://chat/{session}");
        let status = std::process::Command::new("cmd")
            .args(["/C", "start", "", &url])
            .env_remove("ELECTRON_RUN_AS_NODE")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("唤起深链失败")?;
        if !status.success() {
            anyhow::bail!("唤起深链失败：{status}");
        }
        Ok(())
    }

    pub fn list_windows() -> Vec<String> {
        struct Search {
            out: Vec<String>,
        }
        unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let search = unsafe { &mut *(lparam.0 as *mut Search) };
            if unsafe { IsWindowVisible(hwnd) }.as_bool() {
                let mut class = [0u16; 256];
                let clen = unsafe { GetClassNameW(hwnd, &mut class) };
                let class = String::from_utf16_lossy(&class[..clen.max(0) as usize]);
                if class.starts_with("Chrome_WidgetWin") {
                    let mut rect = RECT::default();
                    let _ = unsafe { GetWindowRect(hwnd, &mut rect) };
                    search.out.push(format!(
                        "{class} title={:?} ({}, {}) {}×{} iconic={}",
                        window_title(hwnd),
                        rect.left,
                        rect.top,
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                        unsafe { IsIconic(hwnd) }.as_bool()
                    ));
                }
            }
            BOOL(1)
        }
        let mut search = Search { out: Vec::new() };
        unsafe { EnumWindows(Some(callback), LPARAM(&mut search as *mut Search as isize)) }.ok();
        search.out
    }

    /// 拿输入框节点：主窗口里面积最大的 Edit。
    ///
    /// 必须重试：Chromium 的渲染进程只在「有 UIA 客户端」时才建无障碍树，客户端
    /// 刚连上时拿到的还是退化树（实测 9 个节点、0 个 Edit），等几十毫秒就长全了。
    /// 单次查询会随机失败，这是早点「找不到输入框」的真正原因。
    fn composer() -> Result<(IUIAutomation, IUIAutomationElement)> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
            let hwnd = find_window().ok_or_else(|| anyhow::anyhow!("没找到 WorkBuddy 窗口"))?;
            let condition = automation.CreatePropertyCondition(
                UIA_ControlTypePropertyId,
                &VARIANT::from(UIA_EditControlTypeId.0),
            )?;
            let mut last_count = 0;
            for attempt in 0..8 {
                if attempt > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
                let Ok(root) = automation.ElementFromHandle(hwnd) else {
                    continue;
                };
                let Ok(found) = root.FindAll(TreeScope_Descendants, &condition) else {
                    continue;
                };
                let count = found.Length().unwrap_or(0);
                last_count = count;
                let mut best: Option<(i64, IUIAutomationElement)> = None;
                for i in 0..count {
                    let Ok(element) = found.GetElement(i) else {
                        continue;
                    };
                    let Ok(rect) = element.CurrentBoundingRectangle() else {
                        continue;
                    };
                    let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
                    if w < 200 || h < 20 {
                        continue;
                    }
                    let area = i64::from(w) * i64::from(h);
                    if best.as_ref().is_none_or(|(a, _)| area > *a) {
                        best = Some((area, element));
                    }
                }
                if let Some((_, element)) = best {
                    return Ok((automation, element));
                }
            }
            anyhow::bail!(
                "等了 2 秒仍没拿到输入框（最后一次看到 {last_count} 个小 Edit）。\
                 WorkBuddy 的无障碍树没建起来，先把窗口点到前台再试"
            )
        }
    }

    /// 所有「看起来像 WorkBuddy 主窗口」的候选（可见、类名 Chrome_WidgetWin_1）。
    fn candidates() -> Vec<HWND> {
        struct Search {
            found: Vec<HWND>,
        }
        unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let search = unsafe { &mut *(lparam.0 as *mut Search) };
            if unsafe { IsWindowVisible(hwnd) }.as_bool() && is_candidate(hwnd) {
                search.found.push(hwnd);
            }
            BOOL(1)
        }
        let mut search = Search { found: Vec::new() };
        unsafe { EnumWindows(Some(callback), LPARAM(&mut search as *mut Search as isize)) }.ok();
        search.found
    }

    fn is_candidate(hwnd: HWND) -> bool {
        let mut class = [0u16; 256];
        let len = unsafe { GetClassNameW(hwnd, &mut class) };
        let class = String::from_utf16_lossy(&class[..len.max(0) as usize]);
        class == "Chrome_WidgetWin_1" && window_title(hwnd).contains("WorkBuddy")
    }

    fn window_title(hwnd: HWND) -> String {
        let mut title = [0u16; 512];
        let len = unsafe { GetWindowTextW(hwnd, &mut title) };
        String::from_utf16_lossy(&title[..len.max(0) as usize])
    }

    /// 主窗口：标题就是 `WorkBuddy` 的那个；没有就退回面积最大的候选。
    fn find_window() -> Option<HWND> {
        let all = candidates();
        if let Some(hwnd) = all.iter().find(|h| window_title(**h) == "WorkBuddy") {
            return Some(*hwnd);
        }
        all.into_iter().max_by_key(|hwnd| {
            let mut rect = RECT::default();
            unsafe { GetWindowRect(*hwnd, &mut rect) }.ok();
            i64::from(rect.right - rect.left) * i64::from(rect.bottom - rect.top)
        })
    }

    #[allow(dead_code)]
    fn focus_window() -> Result<()> {
        let hwnd = find_window().ok_or_else(|| anyhow::anyhow!("没找到 WorkBuddy 窗口"))?;
        unsafe {
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
        }
        Ok(())
    }

    /// `mods` 依次按下（不松开），再敲 `key`，最后逆序松开。
    fn key_combo(mods: &[VIRTUAL_KEY], key: VIRTUAL_KEY) -> Result<()> {
        let mut inputs: Vec<INPUT> = Vec::with_capacity(mods.len() * 2 + 2);
        for m in mods {
            inputs.push(key_input(*m, false));
        }
        inputs.push(key_input(key, false));
        inputs.push(key_input(key, true));
        for m in mods.iter().rev() {
            inputs.push(key_input(*m, true));
        }
        let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
        if sent == 0 {
            anyhow::bail!("键盘事件发送失败");
        }
        Ok(())
    }

    fn key_input(key: VIRTUAL_KEY, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: key,
                    wScan: 0,
                    dwFlags: if up {
                        KEYEVENTF_KEYUP
                    } else {
                        Default::default()
                    },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn set_clipboard(text: &str) -> Result<()> {
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes = wide.len() * 2;
        unsafe {
            OpenClipboard(None)?;
            let result = (|| -> Result<()> {
                EmptyClipboard()?;
                let handle = GlobalAlloc(GMEM_MOVEABLE, bytes)?;
                let ptr = GlobalLock(handle);
                if ptr.is_null() {
                    anyhow::bail!("剪贴板内存锁定失败");
                }
                std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr as *mut u16, wide.len());
                let _ = GlobalUnlock(handle);
                // 所有权交给系统，此处不能再 GlobalFree。
                SetClipboardData(CF_UNICODETEXT, windows::Win32::Foundation::HANDLE(handle.0))?;
                Ok(())
            })();
            let _ = CloseClipboard();
            result
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use anyhow::Result;

    pub fn deliver(_text: &str, _send: bool) -> Result<()> {
        anyhow::bail!("语音投递目前只支持 Windows")
    }

    pub fn probe() -> Result<String> {
        anyhow::bail!("语音投递目前只支持 Windows")
    }

    pub fn list_windows() -> Vec<String> {
        vec![]
    }

    pub fn open_session(_session: &str) -> Result<()> {
        anyhow::bail!("语音投递目前只支持 Windows")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_is_16k_mono_pcm16() {
        let dir = std::env::temp_dir().join("pet-voice-test");
        let clip = save_wav(&dir, &[100, -100, 0, 32767]).unwrap();
        let info = describe_wav(&clip.path).unwrap();
        assert!(info.contains("16000 Hz"), "{info}");
        assert!(info.contains("1 声道"), "{info}");
        assert!(info.contains("16 bit"), "{info}");
        assert_eq!(clip.peak, 32767);
    }
}
