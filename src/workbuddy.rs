//! 本机 WorkBuddy 跟随：只读 `~/.workbuddy/projects/<项目>/<会话>.jsonl`。
//!
//! 这是对本地会话文件格式的适配，不是 WorkBuddy 的稳定状态 API。只提取生命周期
//! 元数据（轮次开始、结构化提问、轮次结束、标题），不读取、不保存、不上传任何
//! 对话正文、推理内容、工具参数或代码。
//!
//! 状态映射（全部来自明确记录，不做推测）：
//! - 用户新轮次开始 → 工作
//! - 出现未答复的 `AskUserQuestion` / `ExitPlanMode` 调用 → 等你处理
//! - 助手消息 `status=completed` 且其后无新记录 → 完成
//! - 助手消息 `status` 为中断态 → 已停止
//! - 超时无更新 → 由模型层降级为「状态待确认」，绝不判成功
//!
//! 普通文本提问、失败的工具调用、未知工具都不会被猜成等待或失败。

use crate::model::{clean, Event, ProgressSource, Status};
use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// 单文件首次读取上限：避免启动时阻塞在历史大文件上。
const MAX_INITIAL: u64 = 2 * 1024 * 1024;
/// 最多跟踪的会话文件数，与上游一致。
const MAX_FILES: usize = 24;
/// 只有最近活跃的会话才在首轮导入时上屏，避免历史任务堆满列表。
const IMPORT_WINDOW: i64 = 12 * 3600;
/// 这些工具代表 Agent 正在等待人做选择。
const WAIT_TOOLS: [&str; 2] = ["AskUserQuestion", "ExitPlanMode"];

/// 「需要用户确认」的写入等待要静默这么久才算：这些工具正常执行都在几秒内
/// 完成，卡住超过窗口基本就是弹了「是否允许」的权限确认卡在等用户点。
/// WorkBuddy 的权限请求不写会话 JSONL、日志里也无事件，只能这样从侧面推断。
const PERM_CONFIRM_SECS: i64 = 10;

/// 默认权限模式下会弹确认卡的写入类工具（Bash 在沙箱内自动放行不算）。
fn needs_permission(name: &str) -> bool {
    matches!(
        name,
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" | "NotebookEditCell"
    ) || name.starts_with("mcp__")
}

/// 任务清单写入工具：`todo_write`（本机实际用法）与 `TodoWrite`（兼容写法）。
fn is_todo_write(name: &str) -> bool {
    name.to_ascii_lowercase().replace('_', "") == "todowrite"
}

/// AskUserQuestion 的参数：只取第一问的题干。用户明确要求把这些内容
/// 推上设备屏（知道为什么要回电脑），这是唯一一处取用工具参数的地方，
/// 且只取题干并设长度上限，选项/描述/正文一律不读、不上屏。
///
/// 选项列表刻意不上屏：设备无法替用户点击电脑上的选项（实测模拟键盘与 UIA
/// 都不可行），列出选项只会让人误以为可以在设备上选。
fn ask_text(args: &str) -> String {
    let Ok(v) = serde_json::from_str::<Value>(args) else {
        return String::new();
    };
    let q = &v["questions"][0];
    clean(q["question"].as_str().unwrap_or("").trim(), 60)
}

/// 最近一次工具活动映射成中文状态描述：只看工具名元数据，
/// 不读取、不上屏任何参数或正文。reasoning 不改活动——它太频繁，
/// 会让画面在「思考」和「执行」之间反复闪烁。
fn activity_label(name: &str) -> String {
    let n = name.to_ascii_lowercase().replace(['_', '-'], "");
    let label = match n.as_str() {
        "read" | "view" | "readfile" | "openfile" | "readimage" => "正在阅读文件",
        "edit" | "write" | "multiedit" | "notebookedit" | "strreplace" | "createfile"
        | "editfile" | "writefile" => "正在修改文件",
        "bash" | "powershell" | "shell" | "cmd" | "runcommand" | "executecommand" => "正在执行命令",
        "grep" | "glob" | "search" | "searchdata" | "findfile" => "正在搜索代码",
        "websearch" | "webfetch" | "browser" | "fetchurl" => "正在查阅资料",
        "taskcreate" | "taskupdate" | "tasklist" | "taskget" | "todowrite" | "writetodos" => {
            "正在安排任务"
        }
        "skill" => "正在调用技能",
        "agent" | "task" | "spawnagent" => "正在委派子任务",
        "sendmessage" | "messagecolleague" | "speakinchannel" => "正在协作沟通",
        _ => "正在处理任务",
    };
    label.to_string()
}

/// 「完成」要静默这么久才上报。WorkBuddy 是流式落盘，一轮里 assistant 的
/// completed 后面常常紧跟着下一次工具调用；每 2 秒扫一次就会「完成 → 工作中」
/// 来回刷新（墨水屏全屏闪 + 提示音误响）。静默超过这个窗口才认作真完成。
const DONE_SETTLE: i64 = 8;

/// 系统注入的伪 user 记录：上下文压缩摘要、后台任务通知、模式切换提示等。
/// 它们同样是 role=user，但没有 providerData，会被「非 isMeta 即新需求」的
/// 兜底规则误判成用户又提了一个需求——表现为设备上凭空多出一轮同名任务。
fn is_system_injected(v: &Value) -> bool {
    const TAGS: [&str; 8] = [
        "<task-notification",
        "<conversation_history_summary",
        "<local-command",
        "<user-prompt-submit",
        "<agent-message",
        "<wecom_",
        "<post-tool",
        "<hook",
    ];
    let text = v["content"]
        .as_array()
        .and_then(|a| a.iter().find_map(|c| c["text"].as_str()))
        .or_else(|| v["content"].as_str())
        .unwrap_or("");
    let head = text.trim_start();
    TAGS.iter().any(|tag| head.starts_with(tag))
}

/// 文件尾部记录的类型：状态只由「最后一条记录」决定，而不是由「曾经出现过完成」决定。
#[derive(Clone, Copy, PartialEq, Default)]
enum Last {
    #[default]
    None,
    User,
    AssistantComplete,
    AssistantStopped,
    Activity,
}

#[derive(Default)]
struct Cursor {
    offset: u64,
    /// 末尾尚未写完的半行，必须留到下次读取时拼接，不能丢弃也不能重复消费。
    carry: Vec<u8>,
    task: Option<String>,
    turn: u32,
    /// 会话级标题（WorkBuddy 写的 `ai-title`）。整个会话共用一个，所以它只能当
    /// 兜底——拿它当「当前任务名称」会一直停在会话最早那个名字上。
    title: String,
    /// 本轮用户真正问了什么，逐轮更新，上屏用它。
    turn_title: Option<String>,
    emitted: Option<(Status, String, Option<u8>, String)>,
    last: Last,
    /// callId -> (工具名, 发起记录的时间戳秒)，记录等待型调用；拿到结果即移除。
    waiting: HashMap<String, (String, i64)>,
    /// 本轮任务清单：id -> 是否已完成。只用于算完成项占比，正文一概不保存。
    todos: BTreeMap<String, bool>,
    /// 最近一次工具活动的中文描述（只来自工具名），无清单百分比时上屏用。
    activity: String,
    /// AskUserQuestion 的问题与选项（用户已同意上屏）；等待解除后不再使用。
    asking: String,
    /// 主进程在 main.log 里登记的待确认类型（question/permission），
    /// 与侧边栏「待确认」徽章同源；权限卡不写会话 JSONL，只有这里有信号。
    pending_kind: Option<String>,
    cwd: String,
    /// 最近一条已消费记录的记录时间与序号，静默补报完成时用。
    seen_at: i64,
    seen_seq: u64,
}

/// 大文件只读尾部，标题会落在被跳过的头部；单独扫一段文件头把标题捞回来。
fn head_title(path: &Path) -> Option<String> {
    const HEAD: usize = 256 * 1024;
    let mut f = File::open(path).ok()?;
    let mut buf = vec![0u8; HEAD];
    let n = f.read(&mut buf).ok()?;
    buf.truncate(n);
    let mut title = None;
    for line in buf.split(|b| *b == b'\n') {
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if v["type"].as_str() == Some("ai-title") {
            if let Some(t) = v["aiTitle"].as_str() {
                let t = clean(t.trim(), 60);
                if !t.is_empty() {
                    title = Some(t);
                }
            }
        }
    }
    title
}

/// 去掉宿主注入的 `<system-reminder>…</system-reminder>` 上下文块。
/// 这些块里是模式说明、当前时间、记忆提醒，不是用户问的话。
fn strip_reminders(text: &str) -> String {
    const OPEN: &str = "<system-reminder";
    const CLOSE: &str = "</system-reminder>";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(OPEN) {
        out.push_str(&rest[..start]);
        match rest[start..].find(CLOSE) {
            Some(end) => rest = &rest[start + end + CLOSE.len()..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// 这一轮用户真正问了什么：取提问的第一行，作为「当前任务名称」上屏。
/// 优先读 `<user_query>` 标签里的正文；老格式没有标签时，退回去掉提醒块后的首行。
/// 只取一行且限长——设备画面只有三行标题位，用户自己也不想看一整段。
fn user_title(v: &Value) -> Option<String> {
    let text = v["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if text.trim().is_empty() {
        return None;
    }
    let body = match (text.rfind("<user_query>"), text.rfind("</user_query>")) {
        (Some(a), Some(b)) if b > a => text[a + "<user_query>".len()..b].to_string(),
        _ => strip_reminders(&text),
    };
    let line = body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('<'))?;
    let t = clean(line, 36);
    if t.trim().is_empty() {
        None
    } else {
        Some(t)
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn mtime(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 项目目录名很长（c-Users-… 这类编码串），优先用记录里的 cwd 末段做回退标题。
fn fallback_title(cwd: &str, dir: &Path) -> String {
    let name = Path::new(cwd)
        .file_name()
        .and_then(|x| x.to_str())
        .filter(|x| !x.is_empty())
        .or_else(|| dir.file_name().and_then(|x| x.to_str()))
        .unwrap_or("本机任务");
    format!("WorkBuddy · {}", clean(name, 50))
}

impl Cursor {
    fn feed(&mut self, v: &Value, stem: &str) {
        match v["type"].as_str().unwrap_or("") {
            "session-meta" => {}
            "ai-title" => {
                if let Some(t) = v["aiTitle"].as_str() {
                    let t = clean(t.trim(), 60);
                    if !t.is_empty() {
                        self.title = t;
                    }
                }
            }
            "reasoning" => {
                // 任何一次思考/工具活动都说明这一轮还在进行，完成标记必须让位。
                self.last = Last::Activity;
            }
            "message" => {
                if let Some(cwd) = v["cwd"].as_str() {
                    self.cwd = cwd.to_string();
                }
                match v["role"].as_str().unwrap_or("") {
                    "user" => {
                        // 明确写了 startsNewUserRequest 就照它办：真正的用户消息也会
                        // 带 <system-reminder> 上下文前缀，不能按正文猜。
                        // 只有 providerData 缺失时才走兜底，并先排掉系统注入记录。
                        let new_turn = match v["providerData"]["startsNewUserRequest"].as_bool() {
                            Some(explicit) => explicit,
                            None => {
                                !v["providerData"]["isMeta"].as_bool().unwrap_or(false)
                                    && !is_system_injected(v)
                            }
                        };
                        if new_turn {
                            self.turn += 1;
                            // 任务名跟着这一轮的提问走：会话级 ai-title 一整个
                            // 会话只生成一两次，拿它当任务名会一直停在旧名字上。
                            self.turn_title = user_title(v);
                            // 轮次 ID 用用户消息时间戳：大文件只读尾部时序号会从
                            // 截断点重新计数，与缓存里的旧轮次撞号会被 seq 规则拒绝。
                            // 时间戳天然唯一，跨重启、跨截断都稳定。
                            let ts = v["timestamp"].as_i64().unwrap_or(0);
                            self.task = Some(if ts > 0 {
                                format!("{}-u{}", stem, ts / 1000)
                            } else {
                                format!("{}-t{}", stem, self.turn)
                            });
                            self.last = Last::User;
                            self.waiting.clear();
                            // 清单按轮次计：上一轮的完成项不能算进这一轮。
                            self.todos.clear();
                            self.activity.clear();
                            self.asking.clear();
                            // 新轮必须重新上屏：去重键不含 task_id，否则同标题、
                            // 同状态的下一轮会被当成「和上次一样」直接吞掉。
                            self.emitted = None;
                        }
                    }
                    "assistant" => match v["status"].as_str().unwrap_or("") {
                        // 这是「这条消息完成」，不是「这一轮结束」；是否结束由尾部判定。
                        "completed" => self.last = Last::AssistantComplete,
                        "incomplete" | "canceled" | "cancelled" | "stopped" | "interrupted" => {
                            self.last = Last::AssistantStopped
                        }
                        _ => self.last = Last::Activity,
                    },
                    _ => {}
                }
            }
            "function_call" => {
                self.last = Last::Activity;
                let name = v["name"].as_str().unwrap_or("");
                let id = v["callId"].as_str().unwrap_or("");
                let ts = v["timestamp"].as_i64().unwrap_or(0) / 1000;
                // 活动描述只取工具名；等待型提问不算「活动」，它有自己的状态。
                if !WAIT_TOOLS.contains(&name) {
                    self.activity = activity_label(name);
                }
                if name == "AskUserQuestion" && !id.is_empty() {
                    self.asking = ask_text(v["arguments"].as_str().unwrap_or(""));
                }
                if is_todo_write(name) {
                    self.feed_todos(v["arguments"].as_str().unwrap_or(""));
                }
                if (WAIT_TOOLS.contains(&name) || needs_permission(name)) && !id.is_empty() {
                    self.waiting.insert(id.to_string(), (name.to_string(), ts));
                }
            }
            "function_call_result" => {
                self.last = Last::Activity;
                let id = v["callId"].as_str().unwrap_or("");
                if !id.is_empty() {
                    self.waiting.remove(id);
                }
            }
            _ => {}
        }
    }

    /// 是否处于「需要用户回来处理」：提问类工具立算；写入类工具要卡住超过
    /// PERM_CONFIRM_SECS 才算（正常执行几秒内完成，卡住即权限确认卡在等）。
    fn needs_user(&self, now: i64) -> bool {
        self.waiting.values().any(|(name, ts)| {
            WAIT_TOOLS.contains(&name.as_str())
                || (needs_permission(name) && now.saturating_sub(*ts) >= PERM_CONFIRM_SECS)
        })
    }

    /// 是否有写入确认在等（区别于提问），等待画面据此换文案。
    fn permission_pending(&self, now: i64) -> bool {
        self.waiting.values().any(|(name, ts)| {
            needs_permission(name) && now.saturating_sub(*ts) >= PERM_CONFIRM_SECS
        })
    }

    fn status(&self, now: i64) -> Option<Status> {
        self.task.as_ref()?;
        // 主进程登记的待确认（提问或权限卡）与侧边栏「待确认」徽章同源，
        // 是最权威的信号；会话记录里卡住工具的启发式只作兜底。
        if self.pending_kind.is_some() || self.needs_user(now) {
            return Some(Status::Waiting);
        }
        Some(match self.last {
            Last::AssistantComplete => Status::Done,
            Last::AssistantStopped => Status::Stopped,
            _ => Status::Working,
        })
    }

    /// `todo_write` 是这一轮唯一可核对的进度来源：清单由 Agent 自己写出，
    /// 参数形如 `{"merge":true,"todos":"[{\"id\":\"1\",\"status\":\"completed\"}]"}`，
    /// 注意 `todos` 本身是二次编码的 JSON 字符串。这里只取 id 与 status，
    /// 条目的正文一律不保存、不上屏。
    fn feed_todos(&mut self, args: &str) {
        let Ok(v) = serde_json::from_str::<Value>(args) else {
            return;
        };
        let raw = match v.get("todos") {
            Some(Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
            None => return,
        };
        let Ok(list) = serde_json::from_str::<Value>(&raw) else {
            return;
        };
        let Some(items) = list.as_array() else {
            return;
        };
        let merge = v.get("merge").and_then(Value::as_bool).unwrap_or(false);
        if !merge {
            self.todos.clear();
        }
        for (i, item) in items.iter().enumerate() {
            let id = item["id"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| format!("#{i}"));
            match item["status"].as_str().unwrap_or("") {
                // 取消项不占分母，否则清单永远到不了 100%。
                "cancelled" | "canceled" | "deleted" | "removed" => {
                    self.todos.remove(&id);
                }
                "completed" | "done" => {
                    self.todos.insert(id, true);
                }
                // pending / in_progress / 未知一律算未完成，不猜。
                _ => {
                    self.todos.insert(id, false);
                }
            }
        }
    }

    /// 清单完成度：返回 `(百分比, 已完成, 总数)`；Agent 没写过清单就没有值。
    fn progress(&self) -> Option<(u8, u32, u32)> {
        let total = self.todos.len() as u32;
        if total == 0 {
            return None;
        }
        let done = self.todos.values().filter(|d| **d).count() as u32;
        let pct = ((done * 100 + total / 2) / total).min(100) as u8;
        Some((pct, done, total))
    }

    /// 是否可以把当前状态推上去。「完成」要等记录静默：流式落盘时 assistant 的
    /// completed 后面常紧跟下一次工具调用，太早认完成会让墨水屏来回闪。
    /// 静默用记录时间与当前时间之差衡量——重启后导入的旧记录天然已经静默，
    /// 会立即显示完成，不会被压制。`immediate` 用于首次导入，直接放行。
    fn settled(&mut self, immediate: bool) -> bool {
        if self.status(now_secs()) != Some(Status::Done) || immediate {
            return true;
        }
        now_secs().saturating_sub(self.seen_at) >= DONE_SETTLE
    }

    fn event(&mut self, at: i64, seq: u64, fallback: &str, stem: &str) -> Option<(Event, i64)> {
        // 等待/卡住的判定一律用当前时钟：写入调用发起至今超过窗口才算「卡在
        // 确认卡」，用记录时间算恒为 0，永远到不了等待态。
        let now = now_secs();
        let status = self.status(now)?;
        let task = self.task.clone()?;
        let title = match self.turn_title.as_deref() {
            // 本轮提问优先：这才是「当前任务」的名字。
            Some(t) if !t.trim().is_empty() => t.to_string(),
            // 没有提问正文（老格式会话）时才退到会话标题/项目名。
            _ if self.title.is_empty() => fallback.to_string(),
            _ => self.title.clone(),
        };
        let progress = self.progress();
        // 没有清单百分比时，进度条区域改上最近工具活动的文字描述，
        // 让屏幕始终能回答「现在到底在干什么」。
        let detail = match status {
            // 权限确认卡在等：点明要回电脑点允许（提问题干优先级让位给它，
            // 否则上一问的残留文案会盖住权限提醒）。
            Status::Waiting
                if self.permission_pending(now)
                    || self.pending_kind.as_deref() == Some("permission") =>
            {
                "有写入操作等你确认 · 请回电脑点允许".to_string()
            }
            // 等待选择时把问题一并上屏（用户要求），没有参数内容才用通用文案。
            Status::Waiting if !self.asking.is_empty() => self.asking.clone(),
            Status::Waiting => "WorkBuddy 等待你的选择".to_string(),
            Status::Working if self.activity.is_empty() => String::new(),
            Status::Working => self.activity.clone(),
            _ => String::new(),
        };
        if let Some((s, t, p, d)) = &self.emitted {
            if *s == status && *t == title && *p == progress.map(|x| x.0) && *d == detail {
                return None;
            }
        }
        self.emitted = Some((status, title.clone(), progress.map(|x| x.0), detail.clone()));
        Some((
            Event {
                task_id: task,
                thread_id: Some(stem.to_string()),
                seq,
                status,
                title,
                detail,
                progress: progress.map(|x| x.0),
                progress_source: ProgressSource::Todo,
                progress_steps: progress.map(|x| (x.1, x.2)),
                agent: "WorkBuddy".into(),
            },
            at,
        ))
    }
}

/// WorkBuddy 主进程的任务通知日志（`~/.workbuddy/logs/main.log`）里，
/// 「任务等待确认」有权威事件——侧边栏的「待确认」徽章就是它驱动的：
/// - `[TaskNotify] pending: session=<uuid> kind=question|permission sent=…`
/// - `[TaskNotify] completed: session=<uuid> …`
/// 权限确认卡完全不写会话 JSONL，这是唯一能覆盖所有确认形态的信号源。
#[derive(Default)]
struct TaskNotify {
    /// main.log 已消费到的绝对字节偏移；文件轮转变小则从头重放。
    offset: u64,
    /// 待确认会话：session id -> kind（question / permission）。
    pending: HashMap<String, String>,
}

/// 从 `session=<uuid> …` 片段里取会话 id。正式日志里是 uuid（十六进制与
/// 连字符），但按字母数字放宽解析也无误匹配风险——后面总跟着空格。
fn parse_session_id(rest: &str) -> Option<String> {
    let s = rest.trim_start();
    let end = s
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        .unwrap_or(s.len());
    (!s[..end].is_empty()).then(|| s[..end].to_string())
}

/// 从同一片段里取 `kind=` 的值；没有则返回空串。
fn parse_kind(rest: &str) -> String {
    let Some(i) = rest.find("kind=") else {
        return String::new();
    };
    let s = &rest[i + "kind=".len()..];
    let end = s
        .find(|c: char| !(c.is_ascii_alphabetic() || c == '_'))
        .unwrap_or(s.len());
    s[..end].to_string()
}

impl TaskNotify {
    /// 喂一段日志文本，解析待确认事件。单独拆出来便于单测。
    fn ingest(&mut self, text: &str) {
        for line in text.lines() {
            if let Some(i) = line.find("[TaskNotify] pending: session=") {
                let rest = &line[i + "[TaskNotify] pending: session=".len()..];
                if let Some(id) = parse_session_id(rest) {
                    let kind = parse_kind(rest);
                    if !kind.is_empty() {
                        self.pending.insert(id, kind);
                    }
                }
            } else if let Some(i) = line.find("[TaskNotify] completed: session=") {
                let rest = &line[i + "[TaskNotify] completed: session=".len()..];
                if let Some(id) = parse_session_id(rest) {
                    self.pending.remove(&id);
                }
            }
            // 其余 TaskNotify 行（onEvent/show/auth guide）没有状态语义，忽略。
        }
    }

    /// 增量读取 main.log。尾部半行留到下次；首次接入只读最后 512 KiB，
    /// 不回放整本历史——历史里的 completed 反正只是从空集合里 remove。
    fn poll(&mut self) {
        const TAIL_KEEP: u64 = 512 * 1024;
        let Some(home) = std::env::var_os("USERPROFILE") else {
            return;
        };
        let path = PathBuf::from(home)
            .join(".workbuddy")
            .join("logs")
            .join("main.log");
        let Ok(meta) = std::fs::metadata(&path) else {
            return;
        };
        let size = meta.len();
        if size < self.offset {
            self.offset = 0; // 轮转/重建：重放后集合仍收敛到最新状态
        }
        if self.offset == 0 && size > TAIL_KEEP {
            self.offset = size - TAIL_KEEP;
        }
        let Ok(mut f) = File::open(&path) else {
            return;
        };
        if f.seek(SeekFrom::Start(self.offset)).is_err() {
            return;
        }
        let mut buf = String::new();
        if f.read_to_string(&mut buf).is_err() {
            return;
        }
        self.offset += buf.len() as u64;
        // 末行可能写到一半：残行退回，下次重读。
        if let Some(pos) = buf.rfind('\n') {
            self.offset -= (buf.len() - pos - 1) as u64;
            buf.truncate(pos + 1);
        }
        self.ingest(&buf);
    }
}

pub struct Watcher {
    root: PathBuf,
    cursors: HashMap<PathBuf, Cursor>,
    /// 主进程任务通知日志的待确认集合（question/permission）。
    notify: TaskNotify,
    /// 只看指定会话（stem）：调试与预览单会话状态用，正式运行不设。
    only: Option<String>,
}

impl Watcher {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            cursors: HashMap::new(),
            notify: TaskNotify::default(),
            only: None,
        }
    }

    pub fn only(mut self, stem: &str) -> Self {
        let stem = stem.trim();
        if !stem.is_empty() {
            self.only = Some(stem.to_string());
        }
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 收集 `<项目>/<会话>.jsonl`，只取深度恰好两层，天然排除子目录里的派生产物。
    fn files(&self) -> Result<Vec<(PathBuf, i64)>> {
        let mut out = vec![];
        let projects = std::fs::read_dir(&self.root)
            .with_context(|| format!("未找到 WorkBuddy 会话目录 {}", self.root.display()))?;
        for project in projects.flatten() {
            let dir = project.path();
            if !dir.is_dir() {
                continue;
            }
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("jsonl") && path.is_file() {
                    if let Some(only) = &self.only {
                        if path.file_stem().and_then(|s| s.to_str()) != Some(only.as_str()) {
                            continue;
                        }
                    }
                    out.push((path.clone(), mtime(&path)));
                }
            }
        }
        out.sort_by(|a, b| b.1.cmp(&a.1));
        out.truncate(MAX_FILES);
        Ok(out)
    }

    /// 返回本轮新出现的状态事件；`at` 是记录时间（必要时回退到文件修改时间）。
    pub fn scan(&mut self) -> Result<Vec<(Event, i64)>> {
        let now = now_secs();
        // 先吃掉主进程通知日志：权限确认卡不写会话 JSONL，待确认集合
        // 必须在处理会话文件之前刷新，且无新行的会话也要能升为等待。
        self.notify.poll();
        for (path, c) in self.cursors.iter_mut() {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            c.pending_kind = self.notify.pending.get(stem).cloned();
        }
        let files = self.files()?;
        self.cursors
            .retain(|p, _| files.iter().any(|(f, _)| f == p));
        let mut out = vec![];
        for (path, modified) in files {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("session")
                .to_string();
            let fresh = !self.cursors.contains_key(&path);
            let size = std::fs::metadata(&path)?.len();
            let mut skip_partial = false;
            {
                let cursor = self.cursors.entry(path.clone()).or_default();
                if fresh && size > MAX_INITIAL {
                    // 只读尾部，并且丢掉被截断的首行；标题单独从文件头取回。
                    cursor.offset = size - MAX_INITIAL;
                    skip_partial = true;
                    if let Some(t) = head_title(&path) {
                        cursor.title = t;
                    }
                }
                if size < cursor.offset {
                    // 文件被截断或重建：状态必须重新推导，不能沿用旧进度。
                    let task = cursor.task.take();
                    *cursor = Cursor {
                        task,
                        ..Cursor::default()
                    };
                }
                cursor.pending_kind = self.notify.pending.get(&stem).cloned();
            }
            let mut chunk = vec![];
            {
                let offset = self.cursors[&path].offset;
                let mut f = File::open(&path).context("无法读取 WorkBuddy 会话")?;
                f.seek(SeekFrom::Start(offset))?;
                f.read_to_end(&mut chunk)?;
            }
            if chunk.is_empty() && !fresh {
                continue;
            }
            let c = self.cursors.get_mut(&path).unwrap();
            let carry_before = c.carry.len();
            c.offset += chunk.len() as u64;
            c.carry.extend_from_slice(&chunk);
            // 每条记录的 seq 用它在文件里的绝对字节偏移（+1，避开 0）。
            // 偏移跨进程重启恒定、随追加单调递增，天然幂等；
            // 而自增计数在重启后会从 1 重数与缓存里的旧序号撞车。
            let mut consumed = c.offset - chunk.len() as u64 - carry_before as u64;
            let mut lines: Vec<Vec<u8>> =
                c.carry.split(|b| *b == b'\n').map(|s| s.to_vec()).collect();
            c.carry = lines.pop().unwrap_or_default(); // 末段可能是半行
            if skip_partial && !lines.is_empty() {
                let dropped = lines.remove(0);
                consumed += dropped.len() as u64 + 1;
            }

            let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
            let stale_import = fresh && now.saturating_sub(modified) > IMPORT_WINDOW;
            let mut last_at = modified;
            let mut last_seq = 0u64;
            for line in lines {
                let line_seq = consumed + 1;
                consumed += line.len() as u64 + 1;
                if line.iter().all(|b| b.is_ascii_whitespace()) {
                    continue;
                }
                let Ok(v) = serde_json::from_slice::<Value>(&line) else {
                    continue; // 半行或损坏记录直接跳过，不猜状态
                };
                let at = v["timestamp"]
                    .as_i64()
                    .map(|ms| ms / 1000)
                    .filter(|s| *s > 0)
                    .unwrap_or(modified);
                last_at = at;
                last_seq = line_seq;
                c.feed(&v, &stem);
                c.seen_at = at;
                c.seen_seq = line_seq;
                // 增量扫描必须逐条上报：同一批里可能先后出现「完成」和「下一轮开始」，
                // 合并成一个事件会丢掉完成提示音。首次导入是历史回放，只报最终状态。
                if !fresh && !stale_import && c.settled(false) {
                    let fallback = fallback_title(&c.cwd, &dir);
                    if let Some(ev) = c.event(at, line_seq, &fallback, &stem) {
                        out.push(ev);
                    }
                }
            }

            if fresh {
                let fallback = fallback_title(&c.cwd, &dir);
                if stale_import {
                    // 陈旧会话只建立游标，不上屏：避免把几天前的任务当成当前状态。
                    c.emitted = c.status(now).map(|s| {
                        (
                            s,
                            fallback.clone(),
                            c.progress().map(|p| p.0),
                            c.activity.clone(),
                        )
                    });
                } else if c.settled(true) {
                    if let Some(ev) =
                        c.event(last_at.max(modified), last_seq.max(1), &fallback, &stem)
                    {
                        out.push(ev);
                    }
                }
                if !stale_import {
                    // 记下首扫位置，后续「无新行的补报」（如权限卡升级等待）
                    // 才有依据；陈旧会话刻意不记，避免 idle 循环把它翻出来。
                    c.seen_at = last_at.max(modified);
                    c.seen_seq = last_seq.max(1);
                }
            }
        }

        // 没有新记录时也要有一次判定：被去抖压住的「完成」只有在静默到期后
        // 才轮得到上报，而上面那个循环在没有新行时根本不会执行。
        let idle: Vec<PathBuf> = self.cursors.keys().cloned().collect();
        for path in idle {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("session")
                .to_string();
            let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
            let Some(c) = self.cursors.get_mut(&path) else {
                continue;
            };
            let at = c.seen_at;
            let seq = c.seen_seq;
            if at <= 0 || seq == 0 || !c.settled(false) {
                continue;
            }
            let fallback = fallback_title(&c.cwd, &dir);
            if let Some(ev) = c.event(at, seq, &fallback, &stem) {
                out.push(ev);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Model;
    use serde_json::json;
    use std::io::Write;

    fn temp_root(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("pet4-{tag}-{nanos}"));
        std::fs::create_dir_all(root.join("c-Users-demo-proj")).unwrap();
        root
    }

    fn append(root: &Path, lines: &[Value]) {
        let path = root.join("c-Users-demo-proj").join("sess-0001.jsonl");
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap();
        for l in lines {
            writeln!(f, "{}", serde_json::to_string(l).unwrap()).unwrap();
        }
    }

    fn user(text: &str, ts: i64) -> Value {
        json!({"type":"message","role":"user","timestamp":ts,
               "cwd":"C:/work/demo",
               "content":[{"type":"input_text","text":text}],
               "providerData":{"startsNewUserRequest":true}})
    }

    fn assistant(status: &str, ts: i64) -> Value {
        json!({"type":"message","role":"assistant","status":status,"timestamp":ts,"content":[]})
    }

    /// 上下文压缩摘要：role=user，但没有 providerData，正文是 <conversation_history_summary>。
    fn injected(text: &str, ts: i64) -> Value {
        json!({"type":"message","role":"user","timestamp":ts,
               "cwd":"C:/work/demo",
               "content":[{"type":"input_text","text":text}]})
    }

    #[test]
    fn compaction_summary_does_not_start_a_new_turn() {
        let root = temp_root("compaction");
        let mut w = Watcher::new(root.clone());
        append(&root, &[user("第一轮", 1_700_000_000_000)]);
        let first = w.scan().unwrap();
        let task = first[0].0.task_id.clone();
        append(
            &root,
            &[injected(
                "<conversation_history_summary>\nSummary of the conversation…",
                1_700_000_014_000,
            )],
        );
        let evs = w.scan().unwrap();
        assert!(
            evs.iter().all(|(e, _)| e.task_id == task),
            "压缩摘要不是新需求，不能凭空多出一轮：{:?}",
            evs.iter().map(|(e, _)| &e.task_id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn explicit_new_request_still_wins_over_body_prefix() {
        // 真正的用户消息正文也以 <system-reminder> 开头，靠 providerData 判定。
        let root = temp_root("explicit-turn");
        let mut w = Watcher::new(root.clone());
        append(&root, &[user("第一轮", 1_700_000_000_000)]);
        w.scan().unwrap();
        append(
            &root,
            &[user(
                "<system-reminder data-role=\"user-context\">\n<craft_mode>",
                1_700_000_020_000,
            )],
        );
        let evs = w.scan().unwrap();
        assert!(
            evs.iter().any(|(e, _)| e.task_id.ends_with("-u1700000020")),
            "显式新需求必须开新轮：{:?}",
            evs.iter().map(|(e, _)| &e.task_id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn task_name_comes_from_this_turns_question() {
        // 每轮用户消息前面都挂着一堆 system-reminder，真正的问题在 <user_query> 里。
        let v = user(
            "<system-reminder data-role=\"additional-data\">\n<current_time>\n…\n</current_time>\n</system-reminder>\n<user_query>正在工作下的任务名称为什么与当前任务名称不一致</user_query>",
            1_700_000_000_000,
        );
        assert_eq!(
            user_title(&v).as_deref(),
            Some("正在工作下的任务名称为什么与当前任务名称不一致"),
            "应取提问首行，且不受提醒块干扰"
        );
        // 老格式没有 <user_query>：去掉提醒块后取首个非标签行。
        let v = user(
            "<system-reminder data-role=\"user-context\">\n<craft_mode>\n普通模式\n</craft_mode>\n</system-reminder>\n把项目推上去\n第二行不要",
            1_700_000_001_000,
        );
        assert_eq!(user_title(&v).as_deref(), Some("把项目推上去"));
        // 只有提醒、没有正文：交给上层退回会话标题。
        let v = user(
            "<system-reminder>只有提醒</system-reminder>",
            1_700_000_002_000,
        );
        assert_eq!(user_title(&v), None);
    }

    #[test]
    fn the_screen_name_changes_when_a_new_task_starts() {
        // 会话级 ai-title 只有一个，整段会话不能都叫这个名字。
        let root = temp_root("turn-title");
        let mut w = Watcher::new(root.clone());
        append(
            &root,
            &[
                json!({"type":"ai-title","aiTitle":"查看 Zectrix 本地推送 SDK 项目","timestamp":1}),
                user(
                    "<system-reminder>x</system-reminder>\n<user_query>先看看 SDK</user_query>",
                    1_700_000_000_000,
                ),
            ],
        );
        let first = w.scan().unwrap();
        assert_eq!(first.last().unwrap().0.title, "先看看 SDK");
        append(
            &root,
            &[user(
                "<system-reminder>x</system-reminder>\n<user_query>推送到 GitHub</user_query>",
                1_700_000_060_000,
            )],
        );
        let evs = w.scan().unwrap();
        assert!(
            evs.iter().any(|(e, _)| e.title == "推送到 GitHub"),
            "新一轮的任务名必须换成这一轮的提问：{:?}",
            evs.iter()
                .map(|(e, _)| (e.task_id.clone(), e.title.clone()))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn completion_is_held_until_quiet() {
        let now = now_secs();
        let mut c = Cursor::default();
        c.task = Some("t-demo".into());
        c.last = Last::AssistantComplete;
        // 记录刚写入：完成后面可能还有工具调用，先压住。
        c.seen_at = now;
        assert!(!c.settled(false), "刚落盘的完成不能立刻上报");
        c.seen_at = now - DONE_SETTLE + 1;
        assert!(!c.settled(false), "静默不足");
        c.seen_at = now - DONE_SETTLE;
        assert!(c.settled(false), "静默够了才认完成");
        // 工作中不受去抖影响，也不必等待。
        c.last = Last::Activity;
        c.seen_at = now;
        assert!(c.settled(false), "工作中必须立即上报");
    }

    /// 真实记录里 `arguments` 是字符串，而其中的 `todos` 又是二次编码的 JSON 字符串。
    fn todo_call(items: &[(&str, &str)], merge: bool, ts: i64) -> Value {
        let list: Vec<Value> = items
            .iter()
            .enumerate()
            .map(|(i, (status, id))| json!({"id":id,"status":status,"content":format!("事项{i}")}))
            .collect();
        let args = json!({"merge": merge, "todos": serde_json::to_string(&list).unwrap()});
        json!({"type":"function_call","name":"todo_write","callId":format!("todo-{ts}"),
               "timestamp":ts,"arguments":serde_json::to_string(&args).unwrap()})
    }

    #[test]
    fn todo_list_drives_progress_without_claiming_agent_report() {
        let root = temp_root("todos");
        let mut w = Watcher::new(root.clone());
        append(&root, &[user("带清单的任务", 1_700_000_000_000)]);
        let first = w.scan().unwrap();
        assert_eq!(first[0].0.progress, None, "没有清单时不得编造百分比");

        append(
            &root,
            &[todo_call(
                &[
                    ("completed", "1"),
                    ("in_progress", "2"),
                    ("pending", "3"),
                    ("pending", "4"),
                ],
                true,
                1_700_000_001_000,
            )],
        );
        let evs = w.scan().unwrap();
        let ev = &evs.last().unwrap().0;
        assert_eq!(ev.progress, Some(25));
        assert_eq!(ev.progress_steps, Some((1, 4)), "分母必须可核对");
        assert_eq!(
            ev.progress_source,
            ProgressSource::Todo,
            "清单推算不得标成 Agent 上报"
        );

        // 进度变化本身就是可见变化：状态、标题都没变也必须产出事件，否则不刷新。
        append(
            &root,
            &[todo_call(
                &[
                    ("completed", "1"),
                    ("completed", "2"),
                    ("completed", "3"),
                    ("in_progress", "4"),
                ],
                true,
                1_700_000_002_000,
            )],
        );
        let evs = w.scan().unwrap();
        let ev = &evs.last().unwrap().0;
        assert_eq!(ev.status, Status::Working);
        assert_eq!(ev.progress, Some(75));
    }

    #[test]
    fn cancelled_items_leave_denominator_and_new_turn_resets_list() {
        let root = temp_root("todo-reset");
        let mut w = Watcher::new(root.clone());
        append(&root, &[user("第一轮", 1_700_000_000_000)]);
        w.scan().unwrap();

        append(
            &root,
            &[todo_call(
                &[("completed", "1"), ("completed", "2"), ("cancelled", "3")],
                true,
                1_700_000_001_000,
            )],
        );
        let evs = w.scan().unwrap();
        let ev = &evs.last().unwrap().0;
        assert_eq!(ev.progress_steps, Some((2, 2)), "取消项不占分母");
        assert_eq!(ev.progress, Some(100));

        append(&root, &[user("第二轮", 1_700_000_010_000)]);
        let evs = w.scan().unwrap();
        let ev = &evs.last().unwrap().0;
        assert_eq!(ev.task_id, "sess-0001-u1700000010");
        assert_eq!(ev.progress, None, "新轮次不得继承上一轮的完成项");
    }

    #[test]
    fn maps_real_lifecycle_records_only() {
        let root = temp_root("lifecycle");
        let mut w = Watcher::new(root.clone());
        append(
            &root,
            &[
                json!({"type":"ai-title","aiTitle":"接入宠物状态","timestamp":1_700_000_000_100i64}),
                user("开始", 1_700_000_000_000),
            ],
        );
        let first = w.scan().unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].0.status, Status::Working);
        // 上屏用这一轮的提问，而不是会话级 ai-title（后者整段会话只有一个）。
        assert_eq!(first[0].0.title, "开始");
        // 轮次 ID 来自用户消息时间戳（秒）。
        assert_eq!(first[0].0.task_id, "sess-0001-u1700000000");

        append(&root, &[assistant("completed", 1_700_000_001_000i64)]);
        let done = w.scan().unwrap();
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].0.status, Status::Done);

        append(
            &root,
            &[
                user("再来一轮", 1_700_000_001_500),
                json!({"type":"function_call","name":"AskUserQuestion","callId":"call-1","timestamp":1_700_000_002_000i64}),
            ],
        );
        let waiting = w.scan().unwrap();
        let last = waiting.last().unwrap();
        assert_eq!(last.0.status, Status::Waiting);
        assert_eq!(last.0.task_id, "sess-0001-u1700000001");
        assert!(last.0.detail.contains("等待"));

        append(
            &root,
            &[
                json!({"type":"function_call_result","name":"AskUserQuestion","callId":"call-1","status":"completed","timestamp":1_700_000_003_000i64}),
            ],
        );
        let back = w.scan().unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].0.status, Status::Working);
    }

    #[test]
    fn plain_question_and_failed_tool_are_not_waiting() {
        let root = temp_root("nofalse");
        let mut w = Watcher::new(root.clone());
        append(
            &root,
            &[
                user("提问式对话", 1_700_000_000_000),
                json!({"type":"function_call","name":"Bash","callId":"b-1","timestamp":1_700_000_000_500i64}),
                json!({"type":"function_call_result","name":"Bash","callId":"b-1","status":"error","timestamp":1_700_000_000_600i64}),
                assistant("completed", 1_700_000_001_000i64),
            ],
        );
        let evs = w.scan().unwrap();
        let statuses: Vec<_> = evs.iter().map(|(e, _)| e.status).collect();
        assert_eq!(
            statuses,
            vec![Status::Done],
            "普通提问与失败工具都不得判等待"
        );
    }

    #[test]
    fn new_turn_gets_new_task_id_and_latest_per_chat_wins() {
        let root = temp_root("turns");
        let mut w = Watcher::new(root.clone());
        append(&root, &[user("第一轮", 1_700_000_000_000)]);
        w.scan().unwrap();
        append(
            &root,
            &[
                assistant("completed", 1_700_000_010_000i64),
                user("第二轮", 1_700_000_011_000),
            ],
        );
        let evs = w.scan().unwrap();
        let ids: Vec<_> = evs.iter().map(|(e, _)| e.task_id.clone()).collect();
        assert!(ids.contains(&"sess-0001-u1700000000".to_string()));
        assert!(ids.contains(&"sess-0001-u1700000011".to_string()));

        let mut m = Model::default();
        for (e, at) in evs {
            let _ = m.apply(e, "workbuddy", at);
        }
        assert_eq!(m.live(1_700_000_020).len(), 1, "同一聊天只保留最新一轮");
        assert_eq!(m.current(1_700_000_020).unwrap().status(), Status::Working);
    }

    #[test]
    fn tool_activity_becomes_status_text_without_reading_contents() {
        let root = temp_root("activity");
        let mut w = Watcher::new(root.clone());
        append(
            &root,
            &[
                user("改代码", 1_700_000_000_000),
                json!({"type":"function_call","name":"Bash","callId":"b-1","timestamp":1_700_000_001_000i64}),
            ],
        );
        let evs = w.scan().unwrap();
        let ev = &evs.last().unwrap().0;
        assert_eq!(ev.status, Status::Working);
        assert_eq!(ev.detail, "正在执行命令", "工具名应映射成中文活动描述");

        // 同一类活动持续时不得重复发事件；活动变化才发。
        append(
            &root,
            &[
                json!({"type":"function_call","name":"bash","callId":"b-2","timestamp":1_700_000_002_000i64}),
            ],
        );
        assert!(w.scan().unwrap().is_empty(), "同类活动不重发");

        append(
            &root,
            &[
                json!({"type":"function_call","name":"Grep","callId":"g-1","timestamp":1_700_000_003_000i64}),
            ],
        );
        let evs = w.scan().unwrap();
        assert_eq!(evs.last().unwrap().0.detail, "正在搜索代码");

        // AskUserQuestion 是等待态，不算活动：等待解除后活动描述仍应保留。
        append(
            &root,
            &[
                json!({"type":"function_call","name":"AskUserQuestion","callId":"a-1","timestamp":1_700_000_004_000i64}),
                json!({"type":"function_call_result","name":"AskUserQuestion","callId":"a-1","status":"completed","timestamp":1_700_000_005_000i64}),
            ],
        );
        let evs = w.scan().unwrap();
        let ev = &evs.last().unwrap().0;
        assert_eq!(ev.status, Status::Working);
        assert_eq!(ev.detail, "正在搜索代码", "等待不应清空最近活动");
    }

    #[test]
    fn new_turn_clears_activity_and_done_has_no_activity_text() {
        let root = temp_root("activity-reset");
        let mut w = Watcher::new(root.clone());
        append(
            &root,
            &[
                user("第一轮", 1_700_000_000_000),
                json!({"type":"function_call","name":"Bash","callId":"b-1","timestamp":1_700_000_001_000i64}),
                json!({"type":"message","role":"assistant","status":"completed","timestamp":1_700_000_002_000i64}),
            ],
        );
        let evs = w.scan().unwrap();
        let done = evs.last().unwrap();
        assert_eq!(done.0.status, Status::Done);
        assert_eq!(done.0.detail, "", "完成态不带活动描述");

        append(&root, &[user("第二轮", 1_700_000_010_000)]);
        let evs = w.scan().unwrap();
        let ev = &evs.last().unwrap().0;
        assert_eq!(ev.status, Status::Working);
        assert_eq!(ev.detail, "", "新轮次不得继承上一轮的活动描述");
    }

    #[test]
    fn ask_user_question_shows_prompt_and_options_on_screen() {
        let root = temp_root("ask");
        let mut w = Watcher::new(root.clone());
        let args = json!({"questions":[{"question":"采用哪种推送策略？","header":"Approach",
            "options":[{"label":"只在变更时推送","description":"省电"},{"label":"持续刷新","description":"费电"}]}]});
        append(
            &root,
            &[
                user("要选择", 1_700_000_000_000),
                json!({"type":"function_call","name":"AskUserQuestion","callId":"a-1",
                       "timestamp":1_700_000_001_000i64,
                       "arguments":serde_json::to_string(&args).unwrap()}),
            ],
        );
        let evs = w.scan().unwrap();
        let ev = &evs.last().unwrap().0;
        assert_eq!(ev.status, Status::Waiting);
        assert_eq!(ev.detail, "采用哪种推送策略？", "只上屏题干，不上屏选项");

        // 回答到达后回到工作态，提问内容不再占用 detail。
        append(
            &root,
            &[
                json!({"type":"function_call_result","name":"AskUserQuestion","callId":"a-1","status":"completed","timestamp":1_700_000_002_000i64}),
            ],
        );
        let evs = w.scan().unwrap();
        let ev = &evs.last().unwrap().0;
        assert_eq!(ev.status, Status::Working);
        assert_ne!(ev.detail, "采用哪种推送策略？");
    }

    #[test]
    fn stuck_write_becomes_permission_waiting() {
        let root = temp_root("perm");
        let mut w = Watcher::new(root.clone());
        let now = now_secs();
        // 写入调用刚发起：正常执行中，不算等待。
        append(
            &root,
            &[
                user("写文件", (now - 30) * 1000),
                json!({"type":"function_call","name":"Write","callId":"w-1",
                       "timestamp":(now - 3) * 1000,"arguments":"{}","cwd":"C:/work/demo"}),
            ],
        );
        let evs = w.scan().unwrap();
        assert_eq!(evs.last().unwrap().0.status, Status::Working);

        // 卡住超过窗口（无 function_call_result）：判定为权限确认在等。
        // 先追加一条足够新的活动记录，让卡住窗口覆盖真实时间。
        let mut stale = json!({"type":"function_call","name":"Write","callId":"w-2",
               "timestamp":(now - PERM_CONFIRM_SECS - 2) * 1000,"arguments":"{}","cwd":"C:/work/demo"});
        append(&root, &[stale]);
        let evs = w.scan().unwrap();
        let ev = &evs.last().unwrap().0;
        assert_eq!(ev.status, Status::Waiting, "卡住的写入应升级为等待");
        assert!(
            ev.detail.contains("等你确认"),
            "等待画面应点明是写入确认：{}",
            ev.detail
        );

        // 结果到达（用户点了允许）：立即回到工作态。
        append(
            &root,
            &[
                json!({"type":"function_call_result","callId":"w-2","status":"completed",
                     "timestamp":now * 1000}),
            ],
        );
        let evs = w.scan().unwrap();
        assert_eq!(evs.last().unwrap().0.status, Status::Working);
    }

    #[test]
    fn task_notify_ingest_tracks_pending_sessions() {
        let mut n = TaskNotify::default();
        n.ingest(concat!(
            r#"{"message":["[TaskNotify] onEvent pid=51092 channel=desktopHost:taskPendingNotification"]}"#,
            "\n",
            r#"{"message":["[TaskNotify] pending: session=9e0d76c1-d8df-4501-96ad-dba766f40344 kind=permission sent=false"]}"#,
            "\n",
            r#"{"message":["[TaskNotify] pending: session=abc123 kind=question sent=true"]}"#,
            "\n",
            r#"{"message":["[TaskNotify] completed: session=abc123 wasFocused=true sent=false"]}"#,
            "\n",
            r#"{"message":["[TaskStatusNotifier] show() called title=\"任务等待确认，请处理\""]}"#,
            "\n",
        ));
        assert_eq!(
            n.pending
                .get("9e0d76c1-d8df-4501-96ad-dba766f40344")
                .map(String::as_str),
            Some("permission"),
            "pending 必须按会话 id 与 kind 登记"
        );
        assert!(
            !n.pending.contains_key("abc123"),
            "completed 必须把会话移出待确认集合"
        );
    }

    #[test]
    fn notify_log_permission_upgrades_waiting_without_jsonl() {
        // 关键场景：权限确认卡不写会话 JSONL（截图实测），Bash 命令也不在
        // 写入工具启发式里——只有 main.log 的 TaskNotify 事件能覆盖。
        let root = temp_root("notify-perm");
        let mut w = Watcher::new(root.clone());
        let now = now_secs();
        append(
            &root,
            &[
                user("制作表情包", (now - 60) * 1000),
                json!({"type":"function_call","name":"Bash","callId":"b-1",
                       "timestamp":(now - 20) * 1000,"arguments":"{}","cwd":"C:/work/pkg"}),
            ],
        );
        w.scan().unwrap(); // 建立游标：此刻是工作中

        // 主进程登记待确认，会话记录毫无新行——仍必须升为等待并点明确认。
        w.notify
            .ingest("[TaskNotify] pending: session=sess-0001 kind=permission sent=false");
        let evs = w.scan().unwrap();
        let ev = &evs.last().unwrap().0;
        assert_eq!(ev.status, Status::Waiting, "主进程登记的待确认必须生效");
        assert!(ev.detail.contains("等你确认"), "{}", ev.detail);

        // 用户在电脑上点了允许：JSONL 依旧无新行，也要立即恢复工作态。
        w.notify
            .ingest("[TaskNotify] completed: session=sess-0001 wasFocused=true sent=false");
        let evs = w.scan().unwrap();
        assert_eq!(evs.last().unwrap().0.status, Status::Working);
    }

    #[test]
    fn late_flushed_completion_from_previous_turn_does_not_stick() {
        // WorkBuddy 实测会把上一轮的 assistant completed 延迟补写在下一轮
        // user 记录之后：同一批里「新轮开始 → 完成 → 活动」必须以活动收尾，
        // 且事件序列里完成态之后还能回到工作态（依赖模型层的复活规则）。
        let root = temp_root("late-flush");
        let mut w = Watcher::new(root.clone());
        // 先建立游标（真实守护启动后一直在增量扫描），再模拟延迟补写的一批记录。
        append(&root, &[assistant("completed", 1_700_000_000_000i64)]);
        w.scan().unwrap();
        append(
            &root,
            &[
                user("新需求", 1_700_000_001_000),
                json!({"type":"message","role":"assistant","status":"completed","timestamp":1_700_000_003_000i64}),
                json!({"type":"function_call","name":"Bash","callId":"b-1","timestamp":1_700_000_003_100i64}),
            ],
        );
        let evs = w.scan().unwrap();
        let statuses: Vec<_> = evs.iter().map(|(e, _)| e.status).collect();
        assert_eq!(
            statuses,
            vec![Status::Working, Status::Done, Status::Working],
            "按文件顺序逐条上报，最终状态必须是工作"
        );
        assert_eq!(evs.last().unwrap().0.task_id, "sess-0001-u1700000001");

        let mut m = Model::default();
        for (e, at) in &evs {
            m.apply(e.clone(), "workbuddy", *at).unwrap();
        }
        assert_eq!(
            m.current(1_700_000_004).unwrap().status(),
            Status::Working,
            "补写的完成记录不得把整轮钉死在完成态"
        );
    }

    #[test]
    fn seq_is_file_offset_and_stable_across_restart() {
        // 守护重启后 Watcher 重建，记录同一行必须得到同一个 seq（文件偏移），
        // 否则重放事件会被缓存里的旧序号拒绝，状态就永远停在上次保存的画面。
        let root = temp_root("seq-stable");
        append(&root, &[user("第一轮", 1_700_000_000_000)]);
        let mut a = Watcher::new(root.clone());
        let evs_a = a.scan().unwrap();
        assert!(evs_a[0].0.seq > 0, "seq 必须非 0");

        let mut b = Watcher::new(root.clone());
        let evs_b = b.scan().unwrap();
        assert_eq!(
            evs_a[0].0.seq, evs_b[0].0.seq,
            "同一记录在不同 Watcher 实例中的 seq 必须一致"
        );

        // 重启后的实例继续增量扫描，新记录 seq 必须更大，才能覆盖旧状态。
        append(&root, &[assistant("completed", 1_700_000_010_000i64)]);
        let evs_c = b.scan().unwrap();
        assert!(evs_c[0].0.seq > evs_a[0].0.seq);
        assert_eq!(evs_c[0].0.status, Status::Done);
    }

    #[test]
    fn partial_line_is_held_until_complete() {
        let root = temp_root("partial");
        let path = root.join("c-Users-demo-proj").join("sess-0001.jsonl");
        let mut w = Watcher::new(root.clone());
        let mut f = std::fs::File::create(&path).unwrap();
        write!(
            f,
            "{}",
            serde_json::to_string(&user("半行", 1_700_000_000_000)).unwrap()
        )
        .unwrap();
        f.flush().unwrap();
        assert!(w.scan().unwrap().is_empty(), "半行不得产生事件");
        writeln!(f).unwrap();
        drop(f);
        let evs = w.scan().unwrap();
        assert_eq!(evs.len(), 1, "补全换行后应产生事件");
        assert_eq!(evs[0].0.status, Status::Working);
    }

    #[test]
    fn disabled_follow_hides_local_tasks() {
        let root = temp_root("disabled");
        let mut w = Watcher::new(root.clone());
        append(&root, &[user("任务", 1_700_000_000_000)]);
        let mut m = Model::default();
        for (e, at) in w.scan().unwrap() {
            m.apply(e, "workbuddy", at).unwrap();
        }
        assert!(m.current(1_700_000_020).is_some());
        m.settings.follow_enabled = false;
        assert!(
            m.current(1_700_000_020).is_none(),
            "关闭跟随后本机任务不上屏"
        );
    }
}
