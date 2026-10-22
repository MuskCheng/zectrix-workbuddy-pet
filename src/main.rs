//! ZECTRIX Note 4 像素宠物（WorkBuddy 版）。
//!
//! 复用 zectrix-agent-pet 的状态模型、布局与音效逻辑，把原来的 Hub 运行时替换为
//! 本机光栅化 + zectrix-sdk 局域网直连，并把 Codex 会话跟随换成 WorkBuddy 跟随。

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use workbuddy_pet::{
    device, layout,
    model::{Event as TaskEvent, Model, Status, Task},
    raster::{self, Fonts},
    sound, uia,
    workbuddy::Watcher,
};
use zectrix_sdk::{Device, ImageFormat};

fn now() -> i64 {
    chrono::Local::now().timestamp()
}

fn data_dir() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    let dir = base.join("zectrix-workbuddy-pet");
    // 从早期 workbuddy-pet 版本升级上来时把状态搬过来（只复制、不删源），
    // 这样任务历史与设置不会因为换目录而丢。已有新目录就不动。
    if !dir.join("state.json").exists() {
        let old = base.join("zectrix-workbuddy-pet").join("state.json");
        if old.is_file() {
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::copy(&old, dir.join("state.json"));
        }
    }
    dir
}

fn state_path() -> PathBuf {
    data_dir().join("state.json")
}

fn out_dir() -> PathBuf {
    data_dir().join("out")
}

fn workbuddy_root() -> PathBuf {
    if let Ok(p) = std::env::var("PET4_WORKBUDDY_ROOT") {
        return PathBuf::from(p);
    }
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".workbuddy").join("projects")
}

/// 本机是否还有 WorkBuddy 在跑。只看进程名，不读任何对话内容。
fn workbuddy_running() -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let out = std::process::Command::new("tasklist")
            .args(["/NH", "/FI", "IMAGENAME eq WorkBuddy.exe"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        match out {
            Ok(o) => String::from_utf8_lossy(&o.stdout)
                .to_ascii_lowercase()
                .contains("workbuddy.exe"),
            Err(_) => false,
        }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn startup_dir() -> Option<PathBuf> {
    std::env::var("APPDATA").ok().map(|a| {
        PathBuf::from(a)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("Startup")
    })
}

fn autostart_entry() -> Option<PathBuf> {
    startup_dir().map(|d| d.join("zectrix-workbuddy-pet.vbs"))
}

fn watch_script() -> PathBuf {
    data_dir().join("watch.cmd")
}

/// 自启、计划任务统一指向这份副本：`target\release` 会被 `cargo clean` 抹掉，
/// 拿构建目录当安装目录，早晚会出现「自启项还在、exe 没了」的静默失效。
fn guard_bin() -> PathBuf {
    data_dir().join("bin").join("workbuddy-pet.exe")
}

fn heartbeat_path() -> PathBuf {
    data_dir().join("daemon.hb")
}

fn lock_path() -> PathBuf {
    data_dir().join("daemon.lock")
}

fn supervise_heartbeat_path() -> PathBuf {
    data_dir().join("watchdog.hb")
}

/// 停用标记：`autostart off` 时先落这个文件，`watch.cmd` 的循环看到它就退出。
/// 否则 `off` 杀掉守护后，那个每 10 秒重启一次的批处理还会把它拉回来。
fn stop_flag() -> PathBuf {
    data_dir().join("stopped.flag")
}

/// 心跳多久算新鲜。守护每 5 秒写一次，留 4 倍余量，避免一次磁盘抖动误判为死。
const HEARTBEAT_FRESH: i64 = 20;

/// 看门狗 10 秒一次检查，留同样的余量。
const SUPERVISE_FRESH: i64 = 40;

/// 心跳文件内容：第一行 PID，第二行时间戳。
fn parse_heartbeat(text: &str) -> Option<(u32, i64)> {
    let mut it = text.lines();
    let pid = it.next()?.trim().parse().ok()?;
    let ts = it.next()?.trim().parse().ok()?;
    Some((pid, ts))
}

fn write_heartbeat() {
    let body = format!("{}\n{}", std::process::id(), now());
    // 心跳写失败不影响推送：它只用于「有没有守护在跑」的判断。
    let _ = std::fs::write(heartbeat_path(), body);
}

fn write_supervise_heartbeat() {
    let body = format!("{}\n{}", std::process::id(), now());
    let _ = std::fs::write(supervise_heartbeat_path(), body);
}

/// 心跳是否够新。这条路径被看门狗每 10 秒走一次，所以不查进程表——
/// 只看时间戳，避免每 10 秒拉起一次 tasklist。真正的「进程还在不在」由
/// `daemon.lock` 里的 PID 在启动时判定。
fn heartbeat_fresh(path: &Path, fresh: i64) -> Option<(u32, i64)> {
    let text = std::fs::read_to_string(path).ok()?;
    let (pid, ts) = parse_heartbeat(&text)?;
    (now() - ts <= fresh).then_some((pid, ts))
}

fn guard_alive() -> bool {
    heartbeat_fresh(&heartbeat_path(), HEARTBEAT_FRESH).is_some()
}

fn supervise_alive() -> bool {
    heartbeat_fresh(&supervise_heartbeat_path(), SUPERVISE_FRESH).is_some()
}

/// 更严格的一份判断：心跳新鲜 **且** 该 PID 确实是本程序。
/// 给一次性命令用（`autostart on/status`、`ensure`）——这些场景几十毫秒不算贵，
/// 但结论必须准：刚被结束的进程心跳还会新鲜 20 秒，只看心跳会误报「在运行」。
fn process_running(path: &Path, fresh: i64) -> bool {
    match heartbeat_fresh(path, fresh) {
        Some((pid, _)) => pid_is_ours(pid),
        None => false,
    }
}

fn guard_running() -> bool {
    process_running(&heartbeat_path(), HEARTBEAT_FRESH)
}

fn supervise_running() -> bool {
    process_running(&supervise_heartbeat_path(), SUPERVISE_FRESH)
}

/// 该 PID 是否确实是本程序。只在启动抢锁与 `autostart off` 时调用，频率很低，
/// 所以用 tasklist 而不是引入更多 Win32 依赖。
#[cfg(windows)]
fn pid_is_ours(pid: u32) -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let Ok(out) = std::process::Command::new("tasklist")
        .args(["/NH", "/FI", &format!("PID eq {pid}")])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&out.stdout)
        .to_ascii_lowercase()
        .contains("workbuddy-pet.exe")
}

#[cfg(not(windows))]
fn pid_is_ours(_pid: u32) -> bool {
    false
}

/// 抢占单实例锁：锁里记着真正在跑的那份的 PID。
/// 上一份是崩溃退出的（PID 已消失）就接管，避免 watch.cmd 重启时被自己的
/// 旧锁挡在门外；真有两份同时活着则后来者退出。
fn claim_instance() -> bool {
    if let Ok(text) = std::fs::read_to_string(lock_path()) {
        if let Some((pid, _)) = parse_heartbeat(&text) {
            if pid != std::process::id() && pid_is_ours(pid) {
                return false;
            }
        }
    }
    let _ = std::fs::write(
        lock_path(),
        format!("{}\n{}", std::process::id(), now()),
    );
    true
}

/// 结束某个 PID（`autostart off` 用）。仅当它确实是本程序才动手，
/// 否则心跳里的旧 PID 被系统复用时会误杀别人的进程。
fn stop_pid(pid: u32) -> bool {
    if !pid_is_ours(pid) {
        return false;
    }
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
        unsafe {
            let Ok(handle) = OpenProcess(PROCESS_TERMINATE, false, pid) else {
                return false;
            };
            let ok = TerminateProcess(handle, 0).is_ok();
            let _ = CloseHandle(handle);
            ok
        }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// 停掉守护与看门狗。**先停看门狗**，否则它会在 10 秒内把守护再拉起来。
fn stop_running() {
    for path in [supervise_heartbeat_path(), heartbeat_path()] {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Some((pid, _)) = parse_heartbeat(&text) {
                if stop_pid(pid) {
                    println!("已结束进程 pid {pid}");
                }
            }
        }
        let _ = std::fs::remove_file(&path);
    }
    let _ = std::fs::remove_file(lock_path());
}

/// 拉起守护：优先走 `watch.cmd`（它自带 10 秒重启循环，崩溃能自愈），
/// 没有脚本时直接起本进程的 `watch`。两种方式都在后台、无窗口、脱离当前会话。
/// 常驻进程一律用已安装副本：从 `target\release` 里跑起来的那份，
/// 会在 `cargo clean` 后变成指向空气的进程。
fn installed_exe() -> PathBuf {
    let bin = guard_bin();
    if bin.is_file() {
        bin
    } else {
        std::env::current_exe().unwrap_or(bin)
    }
}

fn append_log() -> Result<(std::fs::File, std::fs::File)> {
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(data_dir().join("watch.log"))?;
    let err = log.try_clone()?;
    Ok((log, err))
}

fn detach(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW 不弹黑框；DETACHED_PROCESS 让子进程不随当前控制台/父进程退出。
        cmd.creation_flags(0x0800_0000 | 0x0000_0008);
    }
}

fn start_guard() -> Result<()> {
    use std::process::{Command, Stdio};

    std::fs::create_dir_all(data_dir())?;
    let (log, err) = append_log()?;
    let mut cmd = Command::new(installed_exe());
    cmd.arg("watch").stdin(Stdio::null()).stdout(log).stderr(err);
    if let Some(t) = load_store().target {
        cmd.arg("--target").arg(t);
    }
    detach(&mut cmd);
    cmd.spawn().context("拉起守护进程失败")?;
    Ok(())
}

/// 拉起看门狗（常驻，发现守护不在就补一份）。
fn start_supervise() -> Result<()> {
    use std::process::{Command, Stdio};
    std::fs::create_dir_all(data_dir())?;
    let (log, err) = append_log()?;
    let mut cmd = Command::new(installed_exe());
    cmd.arg("supervise")
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(err);
    detach(&mut cmd);
    cmd.spawn().context("拉起看门狗失败")?;
    Ok(())
}

fn write_autostart(target: Option<&str>) -> Result<()> {
    let current = std::env::current_exe().context("无法定位 workbuddy-pet.exe")?;
    let log = data_dir().join("watch.log");
    std::fs::create_dir_all(data_dir())?;
    // 旧版本用 watch.cmd + stopped.flag 做自愈，现在由看门狗接管，删掉以免残留生效。
    let _ = std::fs::remove_file(watch_script());
    let _ = std::fs::remove_file(stop_flag());

    // 自启项指向这份副本，不指向构建目录：`cargo clean`、换仓库位置
    // 都会让指向 target\release 的自启项变成「还在，但一启动就找不到 exe」。
    let installed = guard_bin();
    let mut exe = current.clone();
    if current != installed {
        if let Some(dir) = installed.parent() {
            std::fs::create_dir_all(dir)?;
        }
        match std::fs::copy(&current, &installed) {
            Ok(_) => exe = installed.clone(),
            Err(e) if installed.is_file() => {
                // 副本正在被守护用着，Windows 不允许覆盖——沿用旧副本，提示怎么升级。
                eprintln!("无法覆盖已安装副本（{e}），继续使用 {}", installed.display());
                eprintln!("要换版本：先 `workbuddy-pet autostart off` 或结束守护，再执行一次 on。");
                exe = installed.clone();
            }
            Err(e) => {
                eprintln!("复制 exe 到 {} 失败（{e}），自启将指向 {}", installed.display(), current.display());
            }
        }
    }

    let entry = autostart_entry().context("找不到启动文件夹（%APPDATA%）")?;
    // 登录时拉起的是**看门狗**而不是守护本身：看门狗常驻，发现守护不在就补一份，
    // 所以守护被结束、或被更新覆盖后，不用等下一次登录就能回来。
    let vbs = format!(
        "' Generated by \"workbuddy-pet autostart on\": start the watchdog hidden at login.\r\n\
         CreateObject(\"WScript.Shell\").Run \"\"\"{}\"\" supervise\", 0, False\r\n",
        exe.display()
    );
    std::fs::write(&entry, vbs)?;

    if let Some(t) = target.filter(|t| !t.trim().is_empty()) {
        // 记住目标，之后 info/status 与自动发现都不会因为没带参数而失效。
        let mut store = load_store();
        store.target = Some(t.to_string());
        save_store(&store)?;
    }
    println!("已设置自启：{}", entry.display());
    println!("守护程序：{}", exe.display());
    println!("日志：{}", log.display());
    Ok(())
}

fn clear_autostart() -> Result<()> {
    if let Some(entry) = autostart_entry() {
        match std::fs::remove_file(&entry) {
            Ok(()) => println!("已移除自启项：{}", entry.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => println!("没有设置过自启。"),
            Err(e) => return Err(e.into()),
        }
    }
    // 顺序要紧：看门狗先停，否则它会在 10 秒内把守护再拉起来。
    stop_running();
    let _ = std::fs::remove_file(watch_script());
    println!("已停止看门狗与守护进程。");
    Ok(())
}

fn cmd_autostart(args: &Args) -> Result<()> {
    let action = args
        .get("action")
        .or_else(|| args.get("arg0"))
        .unwrap_or("status")
        .to_ascii_lowercase();
    let store = load_store();
    let target = args.get("target").map(str::to_string).or(store.target);
    match action.as_str() {
        "on" | "enable" | "install" => {
            write_autostart(target.as_deref())?;
            // 装完立刻起来，别让用户等到下次登录——「跟随 WorkBuddy」不该有这段空窗。
            if supervise_running() {
                println!("看门狗已在运行，未重复启动。");
            } else {
                start_supervise()?;
                println!("已立即启动看门狗；它会维持守护，WorkBuddy 一出现即可跟随。");
            }
        }
        "off" | "disable" | "uninstall" => clear_autostart()?,
        "status" => {
            let entry = autostart_entry();
            println!(
                "自启项：{}",
                match entry.as_ref().map(|p| p.exists()) {
                    Some(true) => format!("已启用（{}）", entry.unwrap().display()),
                    _ => "未设置".into(),
                }
            );
            println!(
                "守护程序：{}",
                if guard_bin().is_file() {
                    guard_bin().display().to_string()
                } else {
                    "未安装（尚未执行 autostart on）".into()
                }
            );
            match heartbeat_fresh(&supervise_heartbeat_path(), SUPERVISE_FRESH) {
                Some((pid, ts)) if pid_is_ours(pid) => println!(
                    "看门狗：运行中（pid {pid}，心跳 {} 秒前）",
                    now() - ts
                ),
                Some((_, ts)) => println!(
                    "看门狗：已停止（最后心跳 {} 秒前）",
                    now() - ts
                ),
                None => println!("看门狗：未运行"),
            }
            match heartbeat_fresh(&heartbeat_path(), HEARTBEAT_FRESH) {
                Some((pid, ts)) if pid_is_ours(pid) => println!(
                    "守护进程：运行中（pid {pid}，心跳 {} 秒前）",
                    now() - ts
                ),
                Some((_, ts)) => println!(
                    "守护进程：已停止（最后心跳 {} 秒前）",
                    now() - ts
                ),
                None => println!("守护进程：未运行（没有心跳文件）"),
            }
            println!(
                "保存的目标：{}",
                target.unwrap_or_else(|| "（自动发现）".into())
            );
            println!(
                "WorkBuddy 进程：{}",
                if workbuddy_running() {
                    "正在运行"
                } else {
                    "未运行"
                }
            );
        }
        other => anyhow::bail!("未知动作 {other}，可用 on / off / status"),
    }
    Ok(())
}

/// 看门狗：常驻，每隔 10 秒确认守护还在，不在就补一份。
/// 为什么需要它——启动文件夹只在登录那一刻执行一次，进程被结束、被更新覆盖、
/// 或者自启装好之后根本没重新登录，都会出现「WorkBuddy 在跑、却没人推」。
/// 它自己也是本程序，心跳写 `watchdog.hb`，`autostart status` 与 `off` 都看这个。
async fn cmd_supervise(_args: &Args) -> Result<()> {
    if supervise_alive() {
        return Ok(());
    }
    println!(
        "看门狗已启动（每 10 秒检查守护）：{}",
        chrono::Local::now().format("%H:%M:%S")
    );
    let mut tick = tokio::time::interval(Duration::from_secs(10));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        write_supervise_heartbeat();
        if guard_alive() {
            continue;
        }
        match start_guard() {
            Ok(()) => println!(
                "[{}] 守护不在，已拉起",
                chrono::Local::now().format("%H:%M:%S")
            ),
            Err(e) => eprintln!("拉起守护失败：{e}"),
        }
    }
}

/// 一次性检查：守护不在就拉起。给脚本/人工用，幂等，几十毫秒结束。
fn cmd_ensure(args: &Args) -> Result<()> {
    let verbose = args.get("verbose").is_some();
    if guard_running() {
        if verbose {
            println!("守护在运行，无需拉起。");
        }
        return Ok(());
    }
    start_guard()?;
    println!(
        "[{}] 守护不在，已拉起",
        chrono::Local::now().format("%H:%M:%S")
    );
    Ok(())
}


/// 本机持久化：只存状态缓存、设置与设备目标，不含任何会话正文。
#[derive(Clone, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    model: Model,
    #[serde(default)]
    target: Option<String>,
    #[serde(default = "default_format")]
    format: String,
    /// 已经提醒过的指纹（见 `sound::notice_sig`）。持久化是为了让「补提醒」
    /// 在设备超时重连、守护重启后依然不重复响——同一次等待只吵人一次。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    announced: Option<String>,
    /// 守护模式下，最后一个任务结束到交还设备之间保留多久（秒）。规则是
    /// 「任务完成后 5 分钟内没有新任务就解除占用」。`None` = 还没人设过，
    /// 跟随 `default_idle_grace()`——这样「显式设成 300」和「默认 300」能区分开，
    /// 否则写入合并时会把它当成默认值又让磁盘旧值盖回去。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    idle_grace: Option<u32>,
}

/// 任务完成后多久没有新任务就交还设备：5 分钟。
fn default_idle_grace() -> u32 {
    300
}

fn default_format() -> String {
    "gray16".into()
}

impl Default for Store {
    fn default() -> Self {
        Self {
            model: Model::default(),
            target: None,
            format: default_format(),
            announced: None,
            idle_grace: None,
        }
    }
}

impl Store {
    /// 实际生效的宽限秒数：没设过就用默认 5 分钟。
    fn idle_grace(&self) -> i64 {
        self.idle_grace.unwrap_or_else(default_idle_grace) as i64
    }
}

fn load_store() -> Store {
    let path = state_path();
    let mut store = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice::<Store>(&bytes).unwrap_or_else(|e| {
            eprintln!("配置无法解析，使用默认值（原文件未改动）: {e}");
            Store::default()
        }),
        Err(_) => Store::default(),
    };
    // 缓存版本过旧时重建跟随任务；此前从未调用过迁移，旧的序号式轮次 ID
    // 会一直留在缓存里挡住新事件。
    if store.model.migrate_cache() {
        let _ = save_store(&store);
    }
    store
}

fn save_store(store: &Store) -> Result<()> {
    let path = state_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // 守护与交互命令共用一份 state.json。守护内存里的 store 是启动时的快照，
    // 若整份回写，会把别处（init / autostart / run）刚保存的设备地址覆盖成 None。
    // 写入前合并一次：设备地址与格式以「磁盘上的非空值」为准，除非本次显式改了。
    let disk = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Store>(&b).ok());
    let mut out = store.clone();
    if let Some(disk) = disk {
        if out.target.is_none() {
            out.target = disk.target;
        }
        if out.format == default_format() && disk.format != default_format() {
            out.format = disk.format;
        }
        // 提醒指纹是「谁先响过谁说了算」：内存里为空说明这次进程还没响过，
        // 沿用磁盘上的，避免守护重启后把同一条等待又响一遍。
        if out.announced.is_none() {
            out.announced = disk.announced;
        }
        if out.idle_grace.is_none() {
            out.idle_grace = disk.idle_grace;
        }
    }
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(&out)?;
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

struct Args {
    cmd: String,
    flags: Vec<(String, String)>,
}

impl Args {
    fn parse(argv: Vec<String>) -> Self {
        let mut cmd = String::from("help");
        let mut flags = vec![];
        let mut it = argv.into_iter().peekable();
        while let Some(a) = it.next() {
            if let Some(name) = a.strip_prefix("--") {
                let (name, value) = match name.split_once('=') {
                    Some((n, v)) => (n.to_string(), v.to_string()),
                    None => {
                        let next = it.peek().cloned();
                        let value = match next {
                            Some(n) if !n.starts_with("--") => {
                                it.next();
                                n
                            }
                            _ => "true".into(),
                        };
                        (name.to_string(), value)
                    }
                };
                flags.push((name, value));
            } else if cmd == "help" {
                cmd = a;
            } else {
                flags.push((format!("arg{}", flags.len()), a));
            }
        }
        Self { cmd, flags }
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.flags
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    fn has(&self, name: &str) -> bool {
        self.flags.iter().any(|(k, _)| k == name)
    }
}

fn parse_status(value: &str) -> Result<Option<Status>> {
    Ok(match value.trim().to_ascii_lowercase().as_str() {
        "idle" | "none" | "off" => None,
        "working" | "work" => Some(Status::Working),
        "waiting" | "wait" => Some(Status::Waiting),
        "done" | "finish" => Some(Status::Done),
        "error" => Some(Status::Error),
        "stopped" | "stop" => Some(Status::Stopped),
        "unknown" => Some(Status::Unknown),
        other => anyhow::bail!(
            "未知状态 {other}，可用 working / waiting / done / error / stopped / unknown / idle"
        ),
    })
}

/// 读各会话的上下文占用。来源：`~/.workbuddy/workbuddy.db` 的 session_usage 表，
/// `used` 是已用 token、`size` 是窗口大小（如 300000）。
///
/// 直接只读打开会在 WAL 模式下尝试写 db-shm/db-wal（沙箱与只读目录都会失败），
/// 所以先把 db + wal 复制到临时目录再打开副本——库只有几百 KB，代价可忽略。
/// 结果缓存 15 秒：上下文随每轮对话增长，刷新太快只会让墨水屏频繁刷新。
fn read_contexts() -> std::collections::HashMap<String, (u32, u32)> {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    type Map = std::collections::HashMap<String, (u32, u32)>;
    static CACHE: std::sync::OnceLock<Mutex<(Instant, Map)>> = std::sync::OnceLock::new();
    let cache =
        CACHE.get_or_init(|| Mutex::new((Instant::now() - Duration::from_secs(3600), Map::new())));
    let Ok(mut guard) = cache.lock() else {
        return Map::new();
    };
    if guard.0.elapsed() < Duration::from_secs(15) {
        return guard.1.clone();
    }
    let mut map = Map::new();
    if let Ok(home) = std::env::var("USERPROFILE") {
        let src = std::path::PathBuf::from(home)
            .join(".workbuddy")
            .join("workbuddy.db");
        if src.is_file() {
            let tmp = std::env::temp_dir().join("workbuddy-pet-ctx.db");
            let _ = std::fs::remove_file(&tmp);
            let _ = std::fs::remove_file(tmp.with_extension("db-wal"));
            if std::fs::copy(&src, &tmp).is_ok() {
                let _ = std::fs::copy(src.with_extension("db-wal"), tmp.with_extension("db-wal"));
                if let Ok(conn) = rusqlite::Connection::open(&tmp) {
                    let _ = conn.busy_timeout(Duration::from_millis(300));
                    if let Ok(mut stmt) =
                        conn.prepare("select session_id, used, size from session_usage")
                    {
                        if let Ok(rows) = stmt.query_map([], |r| {
                            Ok((
                                r.get::<_, String>(0)?,
                                r.get::<_, i64>(1)?,
                                r.get::<_, i64>(2)?,
                            ))
                        }) {
                            for (id, used, size) in rows.flatten() {
                                if used > 0 && size > 0 {
                                    map.insert(id, (used as u32, size as u32));
                                }
                            }
                        }
                    }
                }
                let _ = std::fs::remove_file(&tmp);
                let _ = std::fs::remove_file(tmp.with_extension("db-wal"));
            }
        }
    }
    *guard = (Instant::now(), map.clone());
    map
}

fn scan_into(model: &mut Model, watcher: &mut Watcher) -> (Vec<TaskEvent>, bool) {
    let mut applied = vec![];
    match watcher.scan() {
        Ok(events) => {
            for (ev, at) in events {
                let keep = ev.clone();
                if model.apply(ev, "workbuddy", at).unwrap_or(false) {
                    applied.push(keep);
                }
            }
        }
        Err(e) => eprintln!("读取 WorkBuddy 会话失败: {e}"),
    }
    let mut changed = !applied.is_empty();
    // 左下角的上下文占用跟着「当前展示的会话」走：切任务、对话变长都能反映出来。
    // 只在整数百分比变化时触发推送——每 1% 刷一次墨水屏已足够，再多就是闪。
    let now = now();
    let ctx = model
        .current(now)
        .and_then(|t| t.event.thread_id)
        .and_then(|id| read_contexts().get(&id).copied());
    let (used, size) = match ctx {
        Some((u, s)) => (Some(u), Some(s)),
        None => (None, None),
    };
    let pct = |u: Option<u32>, s: Option<u32>| {
        u.zip(s).map(|(u, s)| (u as f32 / s as f32 * 100.0) as i32)
    };
    if pct(model.ctx_used, model.ctx_size) != pct(used, size) {
        changed = true;
    }
    model.ctx_used = used;
    model.ctx_size = size;
    (applied, changed)
}

fn render(model: &Model, frame: usize) -> Result<Vec<u8>> {
    let fonts = Fonts::load()?;
    let scene = layout::draw(model, now(), frame);
    raster::to_png(&scene, &fonts)
}

/// 设备占用闸门：只有在「有活跃任务」期间才该占着设备，任务结束后再静默
/// `grace` 秒就交还。墨水屏刷新贵、设备也只有一个连接，长期占着会让别的程序
/// 和设备自己都进不了场；而只看文件、不连设备的待命几乎不花代价。
///
/// 判定用「最后一个任务多久没动静」而不是内存计时器：重启、重连后依然成立，
/// 也天然覆盖「任务在断开期间结束」的情况。超过 `stale_minutes` 没更新的任务
/// 已经算「状态待确认」，同样不再占着设备——否则一张被遗忘的确认卡会锁死设备。
fn may_release(model: &Model, at: i64, grace: i64) -> bool {
    if !model.running(at).is_empty() {
        return false;
    }
    match model.tasks.iter().map(|t| t.updated_at).max() {
        Some(last) => at.saturating_sub(last) >= grace.max(0),
        // 一个任务都没有（比如手动 run --standby）：没有可等的，直接交还。
        None => true,
    }
}

/// 扫描一次会话：只有真实状态变化才落盘、打日志、响提示音。
/// `allow_chime` 用于抑制首次导入历史时的一串提示音。
async fn scan_and_report(
    model: &mut Model,
    watcher: &mut Watcher,
    store: &mut Store,
    dev: &Device,
    allow_chime: bool,
) -> bool {
    let before: Vec<(String, Option<Status>)> = model
        .tasks
        .iter()
        .filter(|t| t.source == "workbuddy")
        .map(|t| (t.event.task_id.clone(), Some(t.status())))
        .collect();
    let (events, changed) = scan_into(model, watcher);
    if changed {
        store.model = model.clone();
        if let Err(e) = save_store(store) {
            eprintln!("配置保存失败: {e}");
        }
    }
    for (i, ev) in events.iter().enumerate() {
        let previous = before
            .iter()
            .find(|(id, _)| *id == ev.task_id)
            .and_then(|(_, s)| *s);
        println!(
            "[状态] {} · {} → {}{}",
            ev.title,
            ev.task_id,
            ev.status.label(),
            ev.progress
                .map(|p| format!("（清单 {p}%）"))
                .unwrap_or_default()
        );
        // 同一批记录里可能先出现「完成」又立刻回到「工作」（上一轮完成记录
        // 延迟补写的场景），提示音只认每个任务的最终状态，中间态不响铃。
        let is_final = !events[i + 1..].iter().any(|e| e.task_id == ev.task_id);
        if is_final && sound::should_chime(allow_chime, previous, ev.status) {
            let target = model
                .tasks
                .iter()
                .find(|t| t.source == "workbuddy" && t.event.task_id == ev.task_id)
                .cloned();
            if let Some(task) = target {
                announce(dev, model, store, &task).await;
            }
        }
    }
    changed
}

/// 只在画面签名变化时推送。签名涵盖状态、标题、进度与当前选中项，
/// 没有真实变化就一个字节都不发——电子墨水屏刷新很贵，也不该无谓闪烁。
/// 把当前模型渲染上屏；画面没变就不重刷。
///
/// 花絮就在这里取：**只有确定要更新页面时**才去 WorkBuddy 界面读一次当前那条，
/// 读到的内容并进同一帧一起推上去。没有独立的定时轮询——界面自己换了十几条
/// 而任务状态没变时，设备不跟着闪屏；一旦要更新页面（状态、进度、占用变化，
/// 或按了键），推上去的就是那一刻界面正在显示的花絮。
async fn push_if_changed(
    dev: &Device,
    model: &mut Model,
    store: &mut Store,
    fonts: &Fonts,
    fmt: ImageFormat,
    frame: usize,
    last_sig: &mut Option<String>,
) -> bool {
    let phase = if model.settings.animation { frame } else { 0 };
    let mut scene = layout::draw(model, now(), phase);
    let mut sig = scene.to_string();
    if last_sig.as_ref() == Some(&sig) {
        return false;
    }
    // 真的要刷新页面了，顺带把界面当刻的花絮取回来，和这一帧同步上屏。
    // 读不到（窗口最小化、气泡已关、UIA 被占）就沿用上一句，绝不阻塞推送。
    if let Some(quip) = uia::current_quip() {
        if model.set_live_quip(&quip) {
            println!("[花絮] {quip}");
            store.model = model.clone();
            if let Err(e) = save_store(store) {
                eprintln!("配置保存失败: {e}");
            }
            scene = layout::draw(model, now(), phase);
            sig = scene.to_string();
            // 花絮不在这一屏上（比如完成页）时，别为了它白刷一次墨水屏。
            if last_sig.as_ref() == Some(&sig) {
                return false;
            }
        }
    }
    let png = match raster::to_png(&scene, fonts) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("渲染失败: {e}");
            return false;
        }
    };
    match dev.show_with_format(&png, fmt).await {
        Ok(()) => match model.current(now()) {
            Some(t) => println!("已推送：{} · {}", t.label(), t.event.title),
            None => println!("已推送：等待任务"),
        },
        // 结果未知时按约定不重发，也不当作失败。
        Err(zectrix_sdk::Error::ResultUnknown) => {
            eprintln!("设备未确认显示结果（ResultUnknown），不重试；请在设备上目检");
        }
        Err(e) => eprintln!("推送失败（不重试）: {e}"),
    }
    // 失败也记录签名：宁可停在旧画面，也不对同一帧反复重刷。
    *last_sig = Some(sig);
    true
}

fn describe(model: &Model) -> String {
    let at = now();
    let current = model.current(at);
    let running = model.running(at);
    let mut out = String::new();
    match &current {
        Some(t) => {
            out.push_str(&format!("当前：{} · {}\n", t.label(), t.event.title));
            out.push_str(&format!("来源：{}", t.source));
            if let Some(p) = t.event.progress {
                match t.event.progress_steps {
                    Some((done, total)) => out.push_str(&format!(" · 清单 {done}/{total}（{p}%）")),
                    None => out.push_str(&format!(" · 上报进度 {p}%")),
                }
            }
            let age = at.saturating_sub(t.updated_at);
            out.push_str(&format!(" · {} 秒前更新\n", age.max(0)));
            if !t.event.detail.is_empty() {
                out.push_str(&format!("详情：{}\n", t.event.detail));
            }
        }
        None => out.push_str("当前：无任务（等待任务）\n"),
    }
    out.push_str(&format!(
        "进行中/等待：{} 个｜已知任务：{} 个｜跟随：{}｜动画：{}｜提示音：{}（音量 {}）\n",
        running.len(),
        model.tasks.len(),
        if model.settings.follow_enabled {
            "开"
        } else {
            "关"
        },
        if model.settings.animation {
            "开"
        } else {
            "关"
        },
        if model.settings.sound_enabled {
            "开"
        } else {
            "关"
        },
        model.settings.volume
    ));
    out
}

fn write_png(path: &Path, png: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, png).with_context(|| format!("无法写入 {}", path.display()))?;
    Ok(())
}

fn label_of(model: &Model) -> String {
    model
        .current(now())
        .map(|t| match t.status() {
            Status::Working => "working",
            Status::Waiting => "waiting",
            Status::Done => "done",
            Status::Error => "error",
            Status::Stopped => "stopped",
            Status::Unknown => "unknown",
        })
        .unwrap_or("idle")
        .to_string()
}

fn with_model(args: &Args, store: &mut Store) -> Model {
    let mut model = store.model.clone();
    match args.get("demo").map(parse_status) {
        Some(Ok(demo)) => {
            // 演示状态只存在于内存，绝不写入任务历史。
            model.demo = demo;
            model
        }
        Some(Err(e)) => {
            eprintln!("{e}");
            model
        }
        None => {
            let mut watcher = Watcher::new(workbuddy_root());
            if let Some(stem) = args.get("session") {
                // 只看这一个会话：否则当前任务会被其它会话更新的记录顶掉。
                let prefix = format!("{stem}-");
                model
                    .tasks
                    .retain(|t| t.source != "workbuddy" || t.event.task_id.starts_with(&prefix));
                watcher = watcher.only(stem);
            }
            scan_into(&mut model, &mut watcher);
            // 单会话预览不落盘：避免把其它会话的任务从状态缓存里抹掉。
            if args.get("session").is_none() {
                store.model = model.clone();
                let _ = save_store(store);
            }
            model
        }
    }
}

async fn cmd_preview(args: &Args) -> Result<()> {
    let mut store = load_store();
    let model = with_model(args, &mut store);
    let frame = args.get("frame").and_then(|f| f.parse().ok()).unwrap_or(0);
    let scene = layout::draw(&model, now(), frame);
    if args.has("scene") {
        println!("{}", serde_json::to_string_pretty(&scene)?);
        return Ok(());
    }
    let png = raster::to_png(&scene, &Fonts::load()?)?;
    let path = args
        .get("out")
        .map(PathBuf::from)
        .unwrap_or_else(|| out_dir().join(format!("pet-{}.png", label_of(&model))));
    write_png(&path, &png)?;
    print!("{}", describe(&model));
    println!("画布：400×300 连续灰阶（未量化、未抖动）");
    println!("预览：{}（{} 字节）", path.display(), png.len());
    Ok(())
}

async fn cmd_sessions(_args: &Args) -> Result<()> {
    let mut watcher = Watcher::new(workbuddy_root());
    println!("会话根目录：{}", watcher.root().display());
    let events = watcher.scan()?;
    if events.is_empty() {
        println!("没有可跟随的会话（最近 12 小时无活动或目录为空）");
    }
    for (e, at) in events {
        let progress = match (e.progress, e.progress_steps) {
            (Some(p), Some((done, total))) => format!("清单 {done}/{total} · {p}%"),
            (Some(p), None) => format!("上报 {p}%"),
            (None, _) => "-".to_string(),
        };
        println!(
            "{:<26} {:<10} {:<18} {}  [{}]",
            e.task_id,
            match e.status {
                Status::Working => "工作",
                Status::Waiting => "等你处理",
                Status::Done => "完成",
                Status::Error => "出错",
                Status::Stopped => "已停止",
                Status::Unknown => "待确认",
            },
            progress,
            e.title,
            at
        );
    }
    Ok(())
}

async fn cmd_check(args: &Args) -> Result<()> {
    let store = load_store();
    let target = args.get("target").map(str::to_string).or(store.target);
    let dev = device::open(target.as_deref()).await?;
    println!(
        "已连接: id={} name={} addr={}",
        dev.summary.id, dev.summary.name, dev.summary.address
    );
    match dev.status().await {
        Ok(s) => println!(
            "电量 {:?}% | 充电 {:?} | RSSI {:?} dBm",
            s.battery_percent, s.charging, s.wifi_rssi_dbm
        ),
        Err(e) => println!("读取状态失败: {e}"),
    }
    let p = dev.palette();
    println!("色板 gray4={:?}", p.gray4);
    println!("色板 gray16={:?}", p.gray16);
    dev.close().await?;
    println!("已断开（未推送任何画面）");
    Ok(())
}

/// 交互确认：读不到输入（管道/非交互）就按「否」处理，不挂住脚本。
fn confirm(prompt: &str) -> bool {
    print!("{prompt} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return false;
    }
    matches!(
        line.trim().to_ascii_lowercase().as_str(),
        "y" | "yes" | "是"
    )
}

/// 首次使用引导：先讲清现状，再连设备、生成预览、按用户确认推送、按需开自启。
/// 每一步都允许失败而不中断——连不上设备仍能看预览、设自启，修正后重跑即可。
async fn cmd_init(args: &Args) -> Result<()> {
    let mut store = load_store();
    println!("WorkBuddy 宠物 · 初始化\n");

    // 1. 现状：把「程序当前认到的配置」先摆出来，用户才能判断哪里不对。
    let sessions = workbuddy_root();
    println!("数据目录      {}", data_dir().display());
    println!(
        "状态缓存      {}（{} 个任务）",
        state_path().display(),
        store.model.tasks.len()
    );
    println!(
        "WorkBuddy    会话 {} ｜进程 {}",
        if sessions.is_dir() {
            "已找到"
        } else {
            "未找到（先启动一次 WorkBuddy）"
        },
        if workbuddy_running() {
            "正在运行"
        } else {
            "未运行"
        }
    );
    println!(
        "已保存设备    {}",
        store.target.as_deref().unwrap_or("（未设置）")
    );
    println!(
        "当前设置      跟随 {}｜提示音 {}（音量 {}）｜动画 {}｜格式 {}",
        if store.model.settings.follow_enabled {
            "开"
        } else {
            "关"
        },
        if store.model.settings.sound_enabled {
            "开"
        } else {
            "关"
        },
        store.model.settings.volume,
        if store.model.settings.animation {
            "开"
        } else {
            "关"
        },
        store.format
    );

    // 2. 设备
    println!("\n[1/3] 连接设备");
    let mut target = args
        .get("target")
        .map(str::to_string)
        .or(store.target.clone());
    // 用户显式写下的地址就有价值：先存起来，连不上也不至于下次还要再输一遍。
    if let Some(t) = args.get("target").map(str::to_string) {
        if store.target.as_deref() != Some(t.as_str()) {
            store.target = Some(t);
            save_store(&store)?;
            println!(
                "  已保存设备地址：{}",
                store.target.as_deref().unwrap_or_default()
            );
        }
    }
    if target.is_none() {
        println!("  未指定地址：自动发现（最多 5 秒）…");
    }
    let mut dev: Option<Device> = None;
    match device::open(target.as_deref()).await {
        Ok(d) => {
            println!(
                "  已连接  id={}  name={}  addr={}",
                d.summary.id, d.summary.name, d.summary.address
            );
            match d.status().await {
                Ok(s) => println!(
                    "  电量 {:?}%｜充电 {:?}｜信号 {:?} dBm",
                    s.battery_percent, s.charging, s.wifi_rssi_dbm
                ),
                Err(e) => println!("  读取设备状态失败：{e}"),
            }
            // 记住地址：之后 watch/run/push 不带 --target 也能用。
            if store.target.as_deref() != Some(d.summary.address.as_str()) {
                store.target = Some(d.summary.address.clone());
                save_store(&store)?;
                println!("  已保存设备地址：{}", d.summary.address);
            }
            target = store.target.clone();
            dev = Some(d);
        }
        Err(e) => {
            // device::open 用 anyhow 包装，多设备要回原始错误类型才能列出候选。
            if let Some(zectrix_sdk::Error::MultipleDevices(list)) =
                e.downcast_ref::<zectrix_sdk::Error>()
            {
                println!("  发现多台设备，请用 --target 指定其一后重跑：");
                for d in list {
                    println!("    {:<16} {:<14} {}", d.address, d.id, d.name);
                }
            } else {
                println!("  连接失败：{e}");
                println!("  逐项检查：① 设备与电脑同一局域网 ② 地址正确");
                println!("            ③ 设备「设置 → 网络同步 → 待机保持 Wi-Fi」已开");
                println!("            ④ 没有别的程序占用（旧守护先停，强杀后等 15 秒）");
            }
        }
    }

    // 3. 预览：先让用户看到要推到设备上的到底是什么。
    println!("\n[2/3] 生成预览");
    let mut watcher = Watcher::new(workbuddy_root());
    scan_into(&mut store.model, &mut watcher);
    let scene = layout::draw(&store.model, now(), 0);
    let png = raster::to_png(&scene, &Fonts::load()?)?;
    let path = out_dir().join("pet-init.png");
    write_png(&path, &png)?;
    print!("{}", describe(&store.model));
    println!(
        "  预览图：{}（400×300，{} 字节）",
        path.display(),
        png.len()
    );

    if let Some(d) = dev {
        if confirm("  先打开上面的预览确认版式，然后把它推到设备？") {
            let fmt = device::format(args.get("format").unwrap_or(&store.format))?;
            match d.show_with_format(&png, fmt).await {
                Ok(()) => println!("  设备已完成显示（{}）", device::format_name(fmt)),
                Err(zectrix_sdk::Error::ResultUnknown) => println!(
                    "  设备未确认结果（ResultUnknown）：按 SDK 约定不重复推送，请在设备上目检"
                ),
                Err(e) => println!("  推送失败：{e}"),
            }
        } else {
            println!("  已跳过推送");
        }
        let _ = d.close().await;
        println!("  已断开连接（不占用设备）");
    }

    // 4. 自启
    println!("\n[3/3] 开机自启");
    if confirm("  登录 Windows 时自动在后台运行？") {
        write_autostart(target.as_deref())?;
    } else {
        println!("  已跳过；之后可运行 `workbuddy-pet autostart on --target <IP>`");
    }

    println!(
        "\n初始化结束。日常使用：`workbuddy-pet watch{}`",
        target.map(|t| format!(" --target {t}")).unwrap_or_default()
    );
    Ok(())
}

async fn cmd_push(args: &Args) -> Result<()> {
    let mut store = load_store();
    let model = with_model(args, &mut store);
    let fmt = device::format(args.get("format").unwrap_or(&store.format))?;
    let png = render(&model, 0)?;
    let target = args
        .get("target")
        .map(str::to_string)
        .or(store.target.clone());
    print!("{}", describe(&model));

    let dev = device::open(target.as_deref()).await?;
    println!(
        "已连接: id={} model={} addr={}",
        dev.summary.id, dev.summary.name, dev.summary.address
    );
    let result = dev.show_with_format(&png, fmt).await;
    let closed = dev.close().await;
    match result {
        Ok(()) => println!(
            "设备已完成显示（{}，{} 字节 PNG）",
            device::format_name(fmt),
            png.len()
        ),
        Err(zectrix_sdk::Error::ResultUnknown) => {
            println!("设备未确认本次显示结果（ResultUnknown）：按 SDK 约定不重试、不重复刷图，请在设备上目检。");
        }
        Err(e) => return Err(anyhow::anyhow!("推送失败: {e}")),
    }
    closed?;
    Ok(())
}

fn write_wav(path: &Path, samples: &[i16], rate: u32) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let data_len = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(samples.len() * 2 + 44);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(data_len + 36).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, bytes)?;
    Ok(())
}

async fn chime(dev: &zectrix_sdk::Device, status: Status, volume: u8) -> bool {
    let audio = device::clip(&sound::pcm(status));
    match dev.play(&audio, volume).await {
        Ok(_) => {
            println!("提示音：已播放（{}）", status.label());
            true
        }
        // 播放失败不重试，切回前台也不补响。
        Err(e) => {
            eprintln!("提示音播放失败（不重试）: {e}");
            false
        }
    }
}

/// 为某个任务提醒一次，并把指纹记到磁盘。
/// 指纹相同就跳过：等待页每 2 秒被重复上报时不会反复响，设备超时重连后
/// 也不会把同一条等待再响一遍；换一张确认卡、换一个提问则指纹不同，会再响。
async fn announce(dev: &Device, model: &Model, store: &mut Store, task: &Task) {
    if !model.settings.sound_enabled {
        return;
    }
    let Some(sig) = sound::notice_sig(task) else {
        return;
    };
    if store.announced.as_deref() == Some(sig.as_str()) {
        return;
    }
    // 只有真的响出来才记指纹：播放失败（设备忙、音频通路未就绪）留到下次补。
    if chime(dev, task.status(), model.settings.volume).await {
        store.announced = Some(sig);
        if let Err(e) = save_store(store) {
            eprintln!("配置保存失败: {e}");
        }
    }
}

async fn cmd_run(args: &Args) -> Result<()> {
    let mut store = load_store();
    if let Some(v) = args.get("volume").and_then(|v| v.parse::<u8>().ok()) {
        store.model.settings.volume = v.min(100);
    }
    if args.has("no-sound") {
        store.model.settings.sound_enabled = false;
    }
    if args.has("no-animation") {
        store.model.settings.animation = false;
    }
    let fmt = device::format(args.get("format").unwrap_or(&store.format))?;
    // 显式地址优先并记住，避免每次都得带 --target。
    if let Some(t) = args.get("target").map(str::to_string) {
        if store.target.as_deref() != Some(t.as_str()) {
            store.target = Some(t);
            save_store(&store)?;
        }
    }
    let target = args
        .get("target")
        .map(str::to_string)
        .or(store.target.clone());
    let mut model = store.model.clone();
    let mut watcher = Watcher::new(workbuddy_root());
    let fonts = Fonts::load()?;

    let dev = device::open(target.as_deref()).await?;
    println!(
        "已连接: id={} model={} addr={}｜灰阶 {}｜Ctrl+C 退出",
        dev.summary.id,
        dev.summary.name,
        dev.summary.address,
        device::format_name(fmt)
    );
    if let Ok(s) = dev.status().await {
        println!(
            "电量 {:?}% | RSSI {:?} dBm",
            s.battery_percent, s.wifi_rssi_dbm
        );
    }

    let watch = args.has("exit-with-workbuddy");
    // 待命模式（守护用）：任务结束就交还设备。手动 `run` 默认一直占着，便于调试。
    let standby = args.has("standby");
    let grace = args
        .get("idle-grace")
        .and_then(|v| v.parse::<u32>().ok())
        .map(|v| v as i64)
        .unwrap_or_else(|| store.idle_grace());
    let mut frame = 0usize;
    let mut last_sig: Option<String> = None;
    let mut scan_tick = tokio::time::interval(Duration::from_secs(2));
    scan_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // 先扫一次再推：否则会把「还没读到会话」的空画面先刷上屏。
    scan_and_report(&mut model, &mut watcher, &mut store, &dev, false).await;
    if push_if_changed(&dev, &mut model, &mut store, &fonts, fmt, frame, &mut last_sig).await {
        frame = (frame + 1) % 6;
    }
    // 首扫静默是为了不把历史回放当成新提醒，但如果此刻屏幕上正停在「等你处理」，
    // 那是一次真正的漏报——设备超时重连、守护重启都会走到这里，等待页会无声出现。
    // 这里补一次提醒；同一提示只响一次（指纹落盘），所以反复重连不会反复响。
    // 完成态只在刚发生（1 分钟内）时补，避免重启后把几小时前的结果当新闻播一遍。
    if let Some(task) = model.current(now()) {
        let status = task.status();
        let fresh = now().saturating_sub(task.updated_at) <= 60;
        if status == Status::Waiting || (status == Status::Done && fresh) {
            announce(&dev, &model, &mut store, &task).await;
        }
    }

    let outcome = loop {
        tokio::select! {
            _ = scan_tick.tick() => {
                if watch && !workbuddy_running() {
                    println!("WorkBuddy 已退出：正常断开设备，回到待命。");
                    break Ok(());
                }
                // 交还设备的判定放在扫描之前：判定用上一拍的状态，
                // grace 以秒计，差一拍不影响，却让「跟随已关闭」时也能正常释放。
                if standby && may_release(&model, now(), grace) {
                    println!("任务已结束（{grace} 秒内没有活跃任务）：释放设备，回到待命。");
                    break Ok(());
                }
                if !model.settings.follow_enabled {
                    continue;
                }
                if scan_and_report(&mut model, &mut watcher, &mut store, &dev, true).await
                    && push_if_changed(&dev, &mut model, &mut store, &fonts, fmt, frame, &mut last_sig)
                        .await
                {
                    frame = (frame + 1) % 6;
                }
            }
            event = dev.next_event() => {
                match event {
                    Ok(Some(zectrix_sdk::Event::Key { key, action })) => {
                        println!("[按键] {key:?} {action:?}");
                        if matches!(action, zectrix_sdk::KeyAction::Short) {
                            match key {
                                // 上下键只在任务之间翻页/选择；选项交互已取消
                                //（设备无法替你点电脑上的选项，等待时只提示回电脑）。
                                zectrix_sdk::Key::Up => model.cycle(-1, now()),
                                zectrix_sdk::Key::Down => model.cycle(1, now()),
                                // 确认 = 回到首页并恢复自动跟随；不批准任何操作。
                                zectrix_sdk::Key::Confirm => model.selected = None,
                            }
                            // 翻页/选项都是用户动作，立刻响应，不等下一次扫描。
                            if push_if_changed(
                                &dev,
                                &mut model,
                                &mut store,
                                &fonts,
                                fmt,
                                frame,
                                &mut last_sig,
                            )
                            .await
                            {
                                frame = (frame + 1) % 6;
                            }
                        }
                    }
                    Ok(Some(zectrix_sdk::Event::RecordingFinished { audio, reason })) => {
                        if let Some(clip) = audio {
                            let name = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
                            let path = out_dir().join("recordings").join(format!("{name}.wav"));
                            match write_wav(&path, &clip.samples, 16_000) {
                                Ok(()) => println!("录音已保存（{reason:?}）：{}", path.display()),
                                Err(e) => eprintln!("录音保存失败: {e}"),
                            }
                        }
                    }
                    Ok(Some(zectrix_sdk::Event::Disconnected { reason })) => {
                        break Err(anyhow::anyhow!("设备连接已结束（{reason:?}），按约定不自动重连"));
                    }
                    Ok(Some(_)) => {}
                    Ok(None) => break Err(anyhow::anyhow!("连接已终止，按约定不自动重连")),
                    Err(e) => break Err(anyhow::anyhow!("事件通道结束: {e}")),
                }
            }
            _ = tokio::signal::ctrl_c() => break Ok(()),
        }
    };

    let closed = dev.close().await;
    store.model = model;
    let _ = save_store(&store);
    println!("已断开并保存状态。");
    closed?;
    outcome
}

/// 待命：只读会话文件，不连设备，等到出现活跃任务（正在工作/等你处理）为止。
/// 返回 `true` = 该去占用设备了；`false` = WorkBuddy 已退出，回外层重新等。
/// 这里刻意不响铃、不推屏：设备还没连上，也还不该被占用。
async fn wait_for_task(store: &mut Store, watcher: &mut Watcher) -> Result<bool> {
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        if !workbuddy_running() {
            println!("WorkBuddy 已退出：结束待命。");
            return Ok(false);
        }
        if !store.model.settings.follow_enabled {
            continue;
        }
        let (events, changed) = scan_into(&mut store.model, watcher);
        if changed {
            if let Err(e) = save_store(store) {
                eprintln!("配置保存失败: {e}");
            }
        }
        for ev in events {
            println!(
                "[状态] {} · {} → {}{}",
                ev.title,
                ev.task_id,
                ev.status.label(),
                ev.progress
                    .map(|p| format!("（清单 {p}%）"))
                    .unwrap_or_default()
            );
        }
        if !store.model.running(now()).is_empty() {
            return Ok(true);
        }
    }
}

/// 守护：WorkBuddy 一启动就进入待命（只读会话、不占设备），一有任务才开始占用
/// 设备推送；任务全部结束并静默 `idle_grace` 秒就交还设备，回到待命。
/// 设备休眠/空闲会主动断开（Timeout），这类断线按 15 秒步长退避重连，
/// 连续失败 5 次才停（避免反复抢占设备）。ResultUnknown 不重发、不抢连接——
/// 这条 SDK 约定只约束「同一次显示请求」，不影响断线后的重新建连。
async fn cmd_watch(args: &Args) -> Result<()> {
    // 单实例：watch.cmd 会在 exe 刚退出时立刻重启，若另一份守护已经在跑，
    // 这里退出——不打印会让「为什么没动静」无从查起，所以留一行带 PID 的痕迹。
    if !claim_instance() {
        println!(
            "[{}] 已有守护在跑，本进程（pid {}）退出",
            chrono::Local::now().format("%H:%M:%S"),
            std::process::id()
        );
        return Ok(());
    }
    println!(
        "[{}] 守护进程启动（pid {}）",
        chrono::Local::now().format("%H:%M:%S"),
        std::process::id()
    );
    // 先落一次心跳，让看门狗/状态查询立刻看到「守护在跑」，不用等第一个 tick。
    write_heartbeat();

    // 显式给了地址就记住：下次 `watch` / `run` / `push` 不带参数也能连上。
    let mut store = load_store();
    if let Some(t) = args.get("target").map(str::to_string) {
        if store.target.as_deref() != Some(t.as_str()) {
            store.target = Some(t);
            save_store(&store)?;
        }
    }
    // 显式给的宽限要记下来（`Some` 才代表用户设过），下次不带参数也照旧。
    if let Some(g) = args.get("idle-grace").and_then(|v| v.parse::<u32>().ok()) {
        if store.idle_grace != Some(g) {
            store.idle_grace = Some(g);
            save_store(&store)?;
        }
    }
    let target = args
        .get("target")
        .map(str::to_string)
        .or_else(|| store.target.clone());
    let grace = store.idle_grace();
    println!(
        "守护模式已启动：目标 {}；有任务才占用设备，{grace} 秒无活跃任务即交还。",
        target.as_deref().unwrap_or("自动发现")
    );
    // 心跳：看门狗、`ensure`、`autostart status` 靠它判断守护在不在。5 秒一次——
    // 心跳过期即视为死亡，间隔越短，被结束后恢复得越快。写失败（磁盘满、权限）
    // 只影响这几个判断，不影响推送。
    tokio::spawn(async {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            write_heartbeat();
        }
    });
    let mut watcher = Watcher::new(workbuddy_root());
    loop {
        if !workbuddy_running() {
            tokio::time::sleep(Duration::from_secs(5)).await;
            continue;
        }
        println!("检测到 WorkBuddy，进入待命（只读会话，不占用设备）。");
        if !wait_for_task(&mut store, &mut watcher).await? {
            tokio::time::sleep(Duration::from_secs(5)).await;
            continue;
        }
        println!("检测到任务，开始占用设备。");
        let mut session = Args {
            cmd: "run".into(),
            flags: args.flags.clone(),
        };
        session
            .flags
            .push(("exit-with-workbuddy".into(), "true".into()));
        session.flags.push(("standby".into(), "true".into()));
        let mut fails = 0u32;
        let mut exhausted = false;
        loop {
            match cmd_run(&session).await {
                // 任务结束交还设备、或 WorkBuddy 退出：都回外层重新待命。
                Ok(()) => break,
                Err(e) => {
                    eprintln!("本次会话异常结束：{e}");
                    // 拿不到设备时别硬抢：已经没有活跃任务就把设备让出来。
                    if load_store().model.running(now()).is_empty() {
                        println!("已无活跃任务，不再重连。");
                        break;
                    }
                    fails += 1;
                    if fails > 5 {
                        eprintln!("连续失败 {fails} 次，暂停重连，等下一个任务再试。");
                        exhausted = true;
                        break;
                    }
                    if !workbuddy_running() {
                        break;
                    }
                    let wait = 15 * fails as u64;
                    println!("{wait} 秒后重连（第 {fails} 次）");
                    tokio::time::sleep(Duration::from_secs(wait)).await;
                }
            }
        }
        if exhausted {
            // 设备可能正在休眠或被人占用：先冷一会儿再回待命，否则
            // 「待命→发现有任务→重连→失败」会立刻转成高频循环。
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    }
}

async fn cmd_sound(args: &Args) -> Result<()> {
    let store = load_store();
    let status = args
        .get("status")
        .map(parse_status)
        .transpose()?
        .flatten()
        .context("需要 --status waiting|done")?;
    let volume = args
        .get("volume")
        .and_then(|v| v.parse::<u8>().ok())
        .unwrap_or(store.model.settings.volume)
        .min(100);
    let target = args.get("target").map(str::to_string).or(store.target);
    let dev = device::open(target.as_deref()).await?;
    println!(
        "已连接: id={} addr={}｜播放 {} 提示音（音量 {volume}%）",
        dev.summary.id,
        dev.summary.address,
        status.label()
    );
    // 实测连接后立刻发音频会被设备以结果码 4 拒绝；留出音频通路初始化时间。
    let settle = args
        .get("settle-ms")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(1500);
    tokio::time::sleep(Duration::from_millis(settle)).await;
    let result = dev.play(&device::clip(&sound::pcm(status)), volume).await;
    let closed = dev.close().await;
    match result {
        Ok(end) => println!("播放结束: {end:?}"),
        Err(e) => return Err(anyhow::anyhow!("播放失败: {e}")),
    }
    closed?;
    Ok(())
}

fn usage() {
    println!(
        r#"ZECTRIX Note 4 像素宠物（WorkBuddy 版）

用法：
  workbuddy-pet init    [--target IP|ID]                           首次使用引导：检查配置 → 连设备 → 预览 → 确认推送 → 自启
  workbuddy-pet preview [--demo <状态>] [--out 路径] [--frame N] [--session <ID>]
                                                          渲染 400×300 预览，不连设备（--session 只看该会话）
  workbuddy-pet scene   [--demo <状态>]                            打印布局 JSON（调试用）
  workbuddy-pet sessions                                           列出可跟随的本机会话、状态与清单进度
  workbuddy-pet check   [--target IP|ID]                           只核对设备，不推画面
  workbuddy-pet sound   --status waiting|done [--target IP|ID] [--volume 0-100]
                                                          只播放一次提示音
  workbuddy-pet quip                                       看一眼此刻从 WorkBuddy 界面读到的花絮（诊断用）
  workbuddy-pet push    [--demo <状态>] [--target IP|ID] [--format gray16|gray4|mono]
                                                          单次推送后立即断开
  workbuddy-pet run     [--target IP|ID] [--format F] [--volume 0-100] [--no-sound] [--no-animation]
                                                          常驻：跟随状态、按键翻页、提示音
  workbuddy-pet watch   [--target IP|ID] [--idle-grace 秒]         守护：WorkBuddy 一启动就待命，有任务才占用设备，
                                                          任务结束 5 分钟内没有新任务即交还并回到待命
  workbuddy-pet autostart on|off|status [--target IP|ID]           开机自启 + 看门狗；on 会装一份副本到数据目录、
                                                          立刻启动守护，并注册 5 分钟一次的检查任务
  workbuddy-pet ensure  [--verbose]                                看门狗入口：WorkBuddy 在跑、守护不在跑就拉起；
                                                          幂等，可安全重复调用
  workbuddy-pet where                                              显示数据与配置路径
  workbuddy-pet forget                                             删除本机状态缓存

状态：working / waiting / done / error / stopped / unknown / idle
  演示状态带「演示」标记，不写入任务历史，也不代表真实任务进度。

设备需与电脑同一局域网，并在设备上开启「设置 → 网络同步 → 待机保持 Wi-Fi」。
"#
    );
}

#[tokio::main]
async fn main() {
    let args = Args::parse(std::env::args().skip(1).collect());
    let cmd = args.cmd.clone();
    let result = match cmd.as_str() {
        "preview" | "scene" => {
            if cmd == "scene" {
                let mut a = args;
                a.flags.push(("scene".into(), "true".into()));
                cmd_preview(&a).await
            } else {
                cmd_preview(&args).await
            }
        }
        "sessions" | "ls" => cmd_sessions(&args).await,
        "init" | "setup" | "guide" => cmd_init(&args).await,
        "check" => cmd_check(&args).await,
        "sound" | "tone" => cmd_sound(&args).await,
        "quip" => {
            // 诊断用：看一眼此刻从 WorkBuddy 界面读到的花絮是什么。
            match uia::current_quip() {
                Some(quip) => println!("界面当前花絮：{quip}"),
                None => println!("没读到花絮（窗口不在、气泡被关掉，或此刻不在任务中）"),
            }
            Ok(())
        }
        "push" | "once" => cmd_push(&args).await,
        "run" | "start" => cmd_run(&args).await,
        "watch" | "daemon" => cmd_watch(&args).await,
        "autostart" | "startup" => cmd_autostart(&args),
        "ensure" | "watchdog" => cmd_ensure(&args),
        "supervise" | "supervisor" => cmd_supervise(&args).await,
        "balance" | "credit" => {
            println!("积分余额是云端数据、本地读不到，该显示项已下线；");
            println!("左下角现在显示当前会话的上下文占用（读 workbuddy.db 的 session_usage）。");
            Ok(())
        }
        "where" => {
            println!("配置: {}", state_path().display());
            println!("输出: {}", out_dir().display());
            println!("会话: {}", workbuddy_root().display());
            Ok(())
        }
        "forget" => {
            let path = state_path();
            match std::fs::remove_file(&path) {
                Ok(()) => println!("已删除 {}", path.display()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    println!("没有已保存的状态。")
                }
                Err(e) => eprintln!("删除失败: {e}"),
            }
            Ok(())
        }
        _ => {
            usage();
            Ok(())
        }
    };
    if let Err(e) = result {
        eprintln!("错误: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use workbuddy_pet::model::ProgressSource;

    fn event(seq: u64, status: Status) -> TaskEvent {
        TaskEvent {
            task_id: "t1".into(),
            thread_id: None,
            seq,
            status,
            title: "真实任务".into(),
            detail: String::new(),
            progress: None,
            progress_source: ProgressSource::Agent,
            progress_steps: None,
            agent: "WorkBuddy".into(),
        }
    }

    fn model_with(seq: u64, status: Status, at: i64) -> Model {
        let mut m = Model::default();
        m.settings.follow_enabled = true;
        m.apply(event(seq, status), "workbuddy", at).unwrap();
        m
    }

    #[test]
    fn releases_only_after_the_grace_period_with_no_active_task() {
        // 任务在跑：不管多久都占着。
        let m = model_with(1, Status::Working, 5_000);
        assert!(!may_release(&m, 5_000, 60));
        assert!(!may_release(&m, 5_300, 60));
        // 任务完成：从完成那一刻起算，满 60 秒才交还。
        let m = model_with(1, Status::Done, 200);
        assert!(!may_release(&m, 200, 60));
        assert!(!may_release(&m, 259, 60));
        assert!(may_release(&m, 260, 60));
    }

    #[test]
    fn a_new_task_cancels_a_pending_release() {
        let mut m = model_with(1, Status::Done, 200);
        assert!(!may_release(&m, 230, 60));
        // 宽限期内用户又发了一条：继续占着，不必断开再重连。
        m.apply(event(2, Status::Working), "workbuddy", 230)
            .unwrap();
        assert!(!may_release(&m, 231, 60));
        assert!(!may_release(&m, 400, 60));
        // 再次结束后重新计时。
        m.apply(event(3, Status::Done), "workbuddy", 410).unwrap();
        assert!(!may_release(&m, 410, 60));
        assert!(may_release(&m, 470, 60));
    }

    #[test]
    fn waiting_holds_the_device_but_an_abandoned_prompt_does_not_lock_it() {
        // 「等你处理」是活跃任务：用户还没回来，必须继续占着设备提醒。
        let m = model_with(1, Status::Waiting, 100);
        assert!(!may_release(&m, 100, 60));
        assert!(!may_release(&m, 800, 60));
        // 超过 stale_minutes（默认 15 分钟）没动静 → 「状态待确认」，不再算活跃；
        // 一张被遗忘的确认卡不能把设备锁到天荒地老。
        assert!(may_release(&m, 2_000, 60));
        // 一直有进展的任务同理：静默超过 15 分钟就交还。
        let m = model_with(1, Status::Working, 100);
        assert!(!may_release(&m, 1_000, 60));
        assert!(may_release(&m, 1_100, 60));
    }

    #[test]
    fn zero_or_absent_grace_releases_immediately() {
        let m = model_with(1, Status::Done, 100);
        assert!(may_release(&m, 100, 0));
        // 负值按 0 处理，不会卡住不交还。
        assert!(may_release(&m, 100, -5));
        // 一个任务都没有（手动 run --standby）：直接交还。
        let m = Model::default();
        assert!(may_release(&m, 100, 60));
    }

    #[test]
    fn default_grace_is_five_minutes_after_the_last_task() {
        // 规则：任务完成后 5 分钟内没有新任务下达就解除占用。
        let store = Store::default();
        assert_eq!(store.idle_grace(), 300);
        let m = model_with(1, Status::Done, 1_000);
        assert!(!may_release(&m, 1_000, store.idle_grace()));
        assert!(!may_release(&m, 1_299, store.idle_grace()));
        assert!(may_release(&m, 1_300, store.idle_grace()));
    }

    #[test]
    fn an_explicit_grace_is_not_mistaken_for_the_default() {
        // 显式设过（Some）就必须以它为准，不能被「等于默认值」误判成没设过，
        // 否则写回 state.json 时会被磁盘旧值盖回去。
        let mut store = Store {
            idle_grace: Some(90),
            ..Store::default()
        };
        assert_eq!(store.idle_grace(), 90);
        // 设成和默认一样的 300 也要能分辨：Some(300) 不等于 None。
        store.idle_grace = Some(300);
        assert!(store.idle_grace.is_some());
    }

    #[test]
    fn heartbeat_parses_pid_and_time() {
        assert_eq!(parse_heartbeat("4242\n1700000000\n"), Some((4242, 1_700_000_000)));
        // 只有一行、或不是数字，都算无效——宁可当成「没有守护」，也不能误判成活着。
        assert_eq!(parse_heartbeat("4242\n"), None);
        assert_eq!(parse_heartbeat(""), None);
        assert_eq!(parse_heartbeat("abc\ndef\n"), None);
    }
}
