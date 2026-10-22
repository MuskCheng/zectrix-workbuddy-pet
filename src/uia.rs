//! 从 WorkBuddy 窗口读「准备中」那一行里的花絮短语。
//!
//! 为什么必须走界面：这条文案是服务端随请求流式下发的，只在渲染进程内存里，
//! 磁盘上没有任何副本。已核查的范围——`app.asar`、`AppData\Roaming\WorkBuddy`
//! 全目录（含 HTTP 缓存、leveldb、Session Storage）、`~/.workbuddy`（会话 JSONL、
//! 日志、workbuddy.db、traces）——用 UTF-8 / UTF-16LE / GBK 三种编码、原样 +
//! gzip + zlib + deflate 四种形态全扫过，一次都没命中。所以守护只读磁盘时
//! 永远拿不到它，只能自己编（即内置短语池）。
//!
//! 取法：UI Automation 读主窗口的可访问性树。实测该窗口是 Electron，
//! `Chrome_WidgetWin_1`，状态行是 `TextControl`：准备阶段为
//! `准备中 你在思考大局，我在打磨细节`，工作中为 `已处理 16m39s`。
//!
//! 采样时机：**只在真要推送时**调一次，不跟着界面持续轮询——墨水屏刷新贵，
//! 而且用户要的就是「推的那一刻界面在说什么」。

use std::sync::mpsc::{self, Sender};
use std::sync::OnceLock;
use std::time::Duration;

/// 采样超时。UIA 是跨进程 COM 调用，窗口卡顿时不能把守护拖住。
const PROBE_TIMEOUT: Duration = Duration::from_millis(700);

/// 读一次当前「准备中」状态行里的花絮；不在准备阶段或读不到时返回 None。
pub fn preparing_quip() -> Option<String> {
    imp::preparing_quip()
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::core::VARIANT;
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    };
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationElement, TreeScope_Descendants,
        UIA_ControlTypePropertyId, UIA_TextControlTypeId,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowRect, GetWindowTextW, IsWindowVisible,
    };

    /// 待命的工作线程：UIA 必须在初始化过 COM 的线程上调用，且一旦常驻，
    /// 后续采样就只剩一次进程内消息投递的代价。
    static CLIENT: OnceLock<Sender<Sender<Option<String>>>> = OnceLock::new();

    pub fn preparing_quip() -> Option<String> {
        let tx = CLIENT.get_or_init(spawn).clone();
        let (reply_tx, reply_rx) = mpsc::channel();
        tx.send(reply_tx).ok()?;
        reply_rx.recv_timeout(PROBE_TIMEOUT).ok().flatten()
    }

    fn spawn() -> mpsc::Sender<Sender<Option<String>>> {
        let (tx, rx) = mpsc::channel::<Sender<Option<String>>>();
        std::thread::Builder::new()
            .name("uia-probe".into())
            .spawn(move || {
                // SAFETY: 线程独占 COM 初始化；失败时下面会自然退化成「读不到」。
                let automation: Option<IUIAutomation> = unsafe {
                    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                    CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()
                };
                for reply in rx {
                    let value = match &automation {
                        Some(a) => read_preparing(a),
                        None => None,
                    };
                    // 调用方可能已经超时走人，发送失败属正常。
                    let _ = reply.send(value);
                }
            })
            .ok();
        tx
    }

    /// 遍历当前会话的状态行，取**最后一条**「准备中…」。
    /// 取最后一条而不是「最后一条带花絮的」：万一界面此刻只有「准备中」没有
    /// 花絮，宁可回退到内置短语池，也不该把历史轮次的旧花絮顶上来。
    fn read_preparing(automation: &IUIAutomation) -> Option<String> {
        let root = element_of_workbuddy(automation)?;
        // 先在服务端按 ControlType 过滤，避免把整棵 Electron 树（400+ 节点）
        // 拉回本地逐个读属性。
        let condition = unsafe {
            automation.CreatePropertyCondition(
                UIA_ControlTypePropertyId,
                &VARIANT::from(UIA_TextControlTypeId.0),
            )
        }
        .ok()?;
        let found = unsafe { root.FindAll(TreeScope_Descendants, &condition) }.ok()?;
        let count = unsafe { found.Length() }.ok()?;
        let mut last_preparing: Option<String> = None;
        for i in 0..count {
            let Ok(element) = (unsafe { found.GetElement(i) }) else {
                continue;
            };
            let Ok(name) = (unsafe { element.CurrentName() }) else {
                continue;
            };
            let line = name.to_string();
            if line.trim_start().starts_with("准备中") {
                last_preparing = Some(line);
            }
        }
        let line = last_preparing?;
        quip_of(&line)
    }

    /// 从 `准备中 你在思考大局，我在打磨细节` 里取出花絮部分。
    pub(super) fn quip_of(line: &str) -> Option<String> {        let rest = line.trim().strip_prefix("准备中")?;
        let quip = rest
            .trim_start_matches([' ', '\u{200b}', '\u{200c}', '·', '|', '/', ':', '：', '-'])
            .trim();
        let len = quip.chars().count();
        // 太短不像花絮（例如只有一个分隔符），太长说明抓到了别的段落。
        if !(4..=40).contains(&len) {
            return None;
        }
        // 花絮是中文拟人短句，至少得有一个汉字，别把「12s」之类当花絮。
        if !quip.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)) {
            return None;
        }
        Some(quip.to_string())
    }

    /// 找 WorkBuddy 主窗口：多进程 Electron，按「可见 + 标题含 WorkBuddy +
    /// 面积最大」挑主窗口（与侧边栏所在的那个一致）。
    fn element_of_workbuddy(automation: &IUIAutomation) -> Option<IUIAutomationElement> {
        struct Search {
            best: Option<(i64, HWND)>,
        }
        unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
            // SAFETY: lparam 由本函数下方传入，始终指向栈上的 Search。
            let search = unsafe { &mut *(lparam.0 as *mut Search) };
            if unsafe { IsWindowVisible(hwnd) }.as_bool() {
                let title = window_title(hwnd);
                if title.contains("WorkBuddy") && class_name(hwnd) == "Chrome_WidgetWin_1" {
                    let mut rect = RECT::default();
                    if unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok() {
                        let area = i64::from(rect.right - rect.left)
                            * i64::from(rect.bottom - rect.top);
                        if search.best.is_none_or(|(best, _)| area > best) {
                            search.best = Some((area, hwnd));
                        }
                    }
                }
            }
            BOOL(1)
        }
        let mut search = Search { best: None };
        // SAFETY: 回调只在本调用期间使用 search；EnumWindows 同步执行。
        unsafe { EnumWindows(Some(callback), LPARAM(&mut search as *mut Search as isize)) }.ok()?;
        let (_, hwnd) = search.best?;
        unsafe { automation.ElementFromHandle(hwnd) }.ok()
    }

    fn window_title(hwnd: HWND) -> String {
        let mut buf = [0u16; 512];
        let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
        String::from_utf16_lossy(&buf[..len.max(0) as usize])
    }

    fn class_name(hwnd: HWND) -> String {
        let mut buf = [0u16; 256];
        let len = unsafe { GetClassNameW(hwnd, &mut buf) };
        String::from_utf16_lossy(&buf[..len.max(0) as usize])
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn preparing_quip() -> Option<String> {
        None
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::imp::quip_of;

    #[test]
    fn takes_only_the_quip_part_of_a_preparing_line() {
        assert_eq!(
            quip_of("准备中 你在思考大局，我在打磨细节").as_deref(),
            Some("你在思考大局，我在打磨细节")
        );
        assert_eq!(
            quip_of("准备中\u{200b}正在给思路抛光打蜡").as_deref(),
            Some("正在给思路抛光打蜡")
        );
    }

    #[test]
    fn rejects_lines_that_are_not_quips() {
        assert_eq!(quip_of("已处理 16m39s"), None, "工作中没有花絮");
        assert_eq!(quip_of("准备中"), None, "只有前缀不算花絮");
        assert_eq!(quip_of("准备中 12s"), None, "时长不是花絮");
    }

    /// 真机联通性测试：窗口不在或不在准备阶段时返回 None 也算通过，
    /// 只验证不 panic、不卡死。`cargo test -- --ignored` 手动跑。
    #[test]
    #[ignore = "需要 WorkBuddy 正在运行"]
    fn probes_live_window_without_panicking() {
        let _ = super::preparing_quip();
    }
}
