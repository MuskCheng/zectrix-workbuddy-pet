use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Working,
    Waiting,
    Done,
    Error,
    Stopped,
    Unknown,
}
impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Working => "正在工作",
            Self::Waiting => "等你处理",
            Self::Done => "任务完成",
            Self::Error => "遇到问题",
            Self::Stopped => "已停止",
            Self::Unknown => "状态待确认",
        }
    }
    pub fn terminal(self) -> bool {
        matches!(self, Self::Done | Self::Error | Self::Stopped)
    }
}

/// 百分比的来源。两者语义不同，界面上必须分开表述：
/// - `Agent`：Agent 自己上报的进度，可直接称为「上报进度」。
/// - `Todo`：从本机会话的任务清单（todo_write）推算的完成项占比，
///   只能称为「清单完成度」，不得冒充 Agent 上报。
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProgressSource {
    #[default]
    Agent,
    Todo,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    pub seq: u64,
    pub status: Status,
    pub title: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub progress: Option<u8>,
    #[serde(default)]
    pub progress_source: ProgressSource,
    /// 清单来源时的 `(已完成, 总数)`，用于显示 `3/7` 这类可核对的分母。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress_steps: Option<(u32, u32)>,
    #[serde(default)]
    pub agent: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Task {
    pub event: Event,
    pub source: String,
    pub updated_at: i64,
    pub stale: bool,
}
impl Task {
    pub fn label(&self) -> &'static str {
        if self.status() == Status::Waiting && self.event.detail.starts_with("权限审批") {
            "等待权限审批"
        } else {
            self.status().label()
        }
    }
    pub fn status(&self) -> Status {
        if self.stale && !self.event.status.terminal() {
            Status::Unknown
        } else {
            self.event.status
        }
    }
    pub fn key(&self) -> String {
        format!("{}:{}", self.source, self.event.task_id)
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Settings {
    pub follow_enabled: bool,
    pub animation: bool,
    pub stale_minutes: u32,
    pub sound_enabled: bool,
    pub volume: u8,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            follow_enabled: true,
            animation: true,
            stale_minutes: 15,
            sound_enabled: true,
            volume: 90,
        }
    }
}
#[derive(Clone, Default, Debug, Deserialize, Serialize)]
pub struct Model {
    #[serde(default)]
    pub cache_version: u32,
    #[serde(default)]
    pub tasks: Vec<Task>,
    #[serde(default)]
    pub settings: Settings,
    /// 当前展示会话的上下文占用（已用 / 窗口），来自 WorkBuddy 的
    /// workbuddy.db `session_usage.used/size`——本地精确可读、实时更新，
    /// 比手填的云端积分余额可靠，也直接决定「还能聊多久」。
    #[serde(default)]
    pub ctx_used: Option<u32>,
    #[serde(default)]
    pub ctx_size: Option<u32>,
    /// 底部花絮短语：WorkBuddy UI 的等待文案是前端随机选的，磁盘上拿不到
    /// 当次显示的那一条，这里用同风格的拟人短语、任务事件变动时轮换。
    #[serde(default)]
    pub quip: String,
    #[serde(skip)]
    pub selected: Option<String>,
    #[serde(skip)]
    pub demo: Option<Status>,
}

/// 拟人花絮短语池。只取动作中的幽默状态，不碰任务内容。
const QUIPS: [&str; 16] = [
    "正在搜肠刮肚，肚子已经刮完了",
    "正在绞尽脑汁，汁水四溅",
    "正在翻箱倒柜找思路",
    "正在脑内疯狂充电",
    "正在敲锣打鼓跑代码",
    "正在低头猛干，勿扰",
    "正在翻阅祖传笔记",
    "正在和 bug 斗智斗勇",
    "正在给思路抛光打蜡",
    "正在榨干最后一点脑力",
    "正在闭关修炼，请勿打扰",
    "正在灵光乍现的边缘试探",
    "正在搬运思想的砖",
    "正在苦思冥想，香火勿断",
    "正在整理脑内书架",
    "正在给灵感浇水",
];

/// 轮换一条与当前不同的短语。用纳秒做伪随机即可——没必要引入 rand。
fn next_quip(current: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as usize)
        .unwrap_or(0);
    let mut idx = nanos % QUIPS.len();
    if QUIPS[idx] == current {
        idx = (idx + 1) % QUIPS.len();
    }
    QUIPS[idx].to_string()
}

/// 花絮短语池（测试与文档用）。
pub fn quip_pool() -> &'static [&'static str] {
    &QUIPS
}
fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 160
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_:.".contains(&b))
}
pub fn clean(s: &str, max: usize) -> String {
    s.chars()
        .filter(|c| {
            !c.is_control() && !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .take(max)
        .collect()
}
pub const TASKS_PER_PAGE: usize = 4;

impl Model {
    /// 重建一次派生的跟随缓存（沿用上游迁移语义）；外部 Agent 事件与设置保留。
    /// 版本 2：轮次 ID 改为时间戳格式（-u<ts>），旧的序号式 ID（-t<N>）全部作废，
    /// 否则尾读重数后的新事件会被旧任务的 seq 挡在门外。
    pub fn migrate_cache(&mut self) -> bool {
        if self.cache_version >= 2 {
            return false;
        }
        self.tasks.retain(|t| t.source != "workbuddy");
        self.cache_version = 2;
        true
    }

    pub fn apply(&mut self, mut event: Event, source: &str, at: i64) -> Result<bool> {
        if !valid_id(&event.task_id) || event.seq == 0 {
            bail!("task_id 须为字母、数字或 -_: .，seq 须大于 0")
        }
        if event.title.trim().is_empty()
            || event.title.chars().count() > 120
            || event.detail.chars().count() > 240
            || event.agent.chars().count() > 40
        {
            bail!("任务标题不能为空，标题/详情/Agent 名称最多 120/240/40 个字符")
        }
        if event.progress.is_some_and(|p| p > 100) {
            bail!("进度必须为 0–100，未知时省略")
        }
        event.title = clean(event.title.trim(), 120);
        event.detail = clean(&event.detail, 240);
        event.agent = clean(&event.agent, 40);
        if event.title.is_empty() {
            bail!("任务标题无有效字符")
        }
        // 记录旧状态：花絮只在状态真正变了才换。同一个「等你处理」每 2 秒
        // 被重复上报（等待期间事件持续产生）时，换花絮会让整屏反复刷新。
        let task_id = event.task_id.clone();
        let before = self
            .tasks
            .iter()
            .find(|t| t.source == source && t.event.task_id == task_id)
            .map(|t| t.status());
        if let Some(t) = self
            .tasks
            .iter_mut()
            .find(|t| t.source == source && t.event.task_id == event.task_id)
        {
            if event.seq < t.event.seq {
                return Ok(false);
            }
            if event.seq == t.event.seq {
                // 同一条记录被重新推导（守护重启后的全新扫描）：记录本身没变，
                // 但派生字段可能变了——最典型的是会话标题在会话中途被重新生成，
                // 重新扫描才第一次读到。内容有变就更新文案，完全一致才去重。
                if event.status == t.event.status
                    && (event.title != t.event.title || event.detail != t.event.detail)
                {
                    t.event.title = event.title;
                    t.event.detail = event.detail;
                    return Ok(true);
                }
                return Ok(false);
            }
            // 会话文件会把上一轮的完成记录延迟补写进下一轮（实测存在）：
            // terminal 任务随后收到更新的非终态事件时必须复活，否则整轮从此静默。
            // 只有「终态改终态」仍然拒绝，外部 Agent 乱报完成态不受影响。
            if t.event.status.terminal() && event.status.terminal() {
                bail!("任务已结束且新事件也是终态；新任务请使用新的 task_id")
            }
            *t = Task {
                event,
                source: source.into(),
                updated_at: at,
                stale: false,
            };
        } else {
            self.tasks.push(Task {
                event,
                source: source.into(),
                updated_at: at,
                stale: false,
            });
        }
        self.tasks
            .sort_by_key(|t| std::cmp::Reverse((t.updated_at, t.event.seq)));
        self.tasks.truncate(100);
        // 状态真的变了才换花絮：同一状态的心跳/等待重复上报不再改动画面。
        if before != Some(self.status_of(&task_id, source)) {
            self.quip = next_quip(&self.quip);
        }
        Ok(true)
    }

    /// 按 task_id 查当前状态（花絮轮换判定用）。
    fn status_of(&self, task_id: &str, source: &str) -> Status {
        self.tasks
            .iter()
            .find(|t| t.source == source && t.event.task_id == task_id)
            .map(|t| t.status())
            .unwrap_or(Status::Unknown)
    }
    pub fn visible(&self, now: i64) -> Vec<Task> {
        let mut tasks: Vec<_> = self
            .tasks
            .iter()
            .filter(|t| t.source != "workbuddy" || self.settings.follow_enabled)
            .cloned()
            .collect();
        tasks.sort_by_key(|t| std::cmp::Reverse((t.updated_at, t.event.seq)));
        for t in &mut tasks {
            if now.saturating_sub(t.updated_at) > self.settings.stale_minutes as i64 * 60 {
                t.stale = true;
            }
        }
        tasks
    }
    /// 每个聊天只保留最新一轮；旧的未结束轮次不得并排显示为并行工作。
    pub fn live(&self, now: i64) -> Vec<Task> {
        let mut seen = std::collections::HashSet::new();
        self.visible(now)
            .into_iter()
            .filter(|t| {
                if t.source == "workbuddy" {
                    if let Some(id) = &t.event.thread_id {
                        if !seen.insert(id.clone()) {
                            return false;
                        }
                    }
                }
                true
            })
            .collect()
    }
    pub fn running(&self, now: i64) -> Vec<Task> {
        let mut tasks: Vec<_> = self
            .live(now)
            .into_iter()
            .filter(|t| matches!(t.status(), Status::Working | Status::Waiting))
            .collect();
        // Heartbeats and waiting transitions must not move a task between rows.
        tasks.sort_by(|a, b| {
            (
                &a.source,
                a.event.thread_id.as_ref().unwrap_or(&a.event.task_id),
            )
                .cmp(&(
                    &b.source,
                    b.event.thread_id.as_ref().unwrap_or(&b.event.task_id),
                ))
        });
        tasks
    }
    pub fn current(&self, now: i64) -> Option<Task> {
        if let Some(status) = self.demo {
            return Some(Task {
                event: Event {
                    task_id: "demo".into(),
                    thread_id: None,
                    seq: 1,
                    status,
                    title: "整理今天的待办".into(),
                    detail: "状态演示，不代表真实任务".into(),
                    progress: if status == Status::Working {
                        Some(60)
                    } else {
                        None
                    },
                    progress_source: ProgressSource::Agent,
                    progress_steps: None,
                    agent: "演示".into(),
                },
                source: "demo".into(),
                updated_at: now,
                stale: false,
            });
        }
        let active = self.running(now);
        let tasks = if active.len() > 1 {
            active
        } else {
            self.visible(now)
        };
        if let Some(key) = &self.selected {
            if let Some(t) = tasks.iter().find(|t| &t.key() == key) {
                return Some(t.clone());
            }
        }
        let tasks = self.live(now);
        tasks
            .iter()
            .find(|t| t.status() == Status::Waiting)
            .or_else(|| tasks.iter().find(|t| t.status() == Status::Working))
            .or_else(|| tasks.first())
            .cloned()
    }
    /// 当前任务是否需要用户回电脑处理（等待选择）。
    pub fn needs_action(&self, now: i64) -> bool {
        self.current(now)
            .map(|t| t.status() == Status::Waiting)
            .unwrap_or(false)
    }
    pub fn cycle(&mut self, step: i32, now: i64) {
        self.demo = None;
        // 单个活跃任务时上下键无事可做：翻到历史任务只会让人困惑。
        let active = self.running(now);
        if active.len() <= 1 {
            return;
        }
        if active.len() > TASKS_PER_PAGE {
            let page = self
                .selected
                .as_ref()
                .and_then(|key| active.iter().position(|t| &t.key() == key))
                .unwrap_or(0)
                / TASKS_PER_PAGE;
            let pages = active.len().div_ceil(TASKS_PER_PAGE);
            let next = (page as i32 + step).rem_euclid(pages as i32) as usize;
            self.selected = Some(active[next * TASKS_PER_PAGE].key());
            return;
        }
        let tasks = active;
        if tasks.is_empty() {
            self.selected = None;
            return;
        }
        let key = self.current(now).map(|t| t.key());
        let idx = tasks.iter().position(|t| Some(t.key()) == key).unwrap_or(0) as i32;
        self.selected = Some(tasks[(idx + step).rem_euclid(tasks.len() as i32) as usize].key());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ev(seq: u64, status: Status) -> Event {
        Event {
            task_id: "test".into(),
            thread_id: None,
            seq,
            status,
            title: "真实任务".into(),
            detail: String::new(),
            progress: None,
            progress_source: ProgressSource::Agent,
            progress_steps: None,
            agent: "Agent".into(),
        }
    }
    #[test]
    fn rebuilds_legacy_cache_once_without_losing_agent_tasks_or_settings() {
        let mut m = Model::default();
        m.settings.follow_enabled = true;
        m.settings.volume = 35;
        m.apply(ev(1, Status::Stopped), "workbuddy", 100).unwrap();
        m.apply(ev(1, Status::Done), "agent", 100).unwrap();
        assert!(m.migrate_cache());
        assert_eq!(m.tasks.len(), 1);
        assert_eq!(m.tasks[0].source, "agent");
        assert!(m.settings.follow_enabled);
        assert_eq!(m.settings.volume, 35);
        m.apply(ev(2, Status::Working), "workbuddy", 101).unwrap();
        let mut restored: Model = serde_json::from_slice(&serde_json::to_vec(&m).unwrap()).unwrap();
        assert!(!restored.migrate_cache());
        assert_eq!(restored.tasks.len(), 2);
    }
    #[test]
    fn running_shows_multiple_chats_but_not_old_turns_or_stale_work() {
        let mut m = Model::default();
        m.settings.follow_enabled = true;
        let mut e = ev(1, Status::Working);
        e.thread_id = Some("chat-a".into());
        m.apply(e.clone(), "workbuddy", 10).unwrap();
        e.task_id = "new-turn".into();
        e.seq = 2;
        e.status = Status::Done;
        m.apply(e.clone(), "workbuddy", 10).unwrap();
        assert!(m.running(11).is_empty());
        e.task_id = "other".into();
        e.thread_id = Some("chat-b".into());
        e.status = Status::Working;
        m.apply(e.clone(), "workbuddy", 12).unwrap();
        e.task_id = "third".into();
        e.thread_id = Some("chat-c".into());
        e.status = Status::Waiting;
        m.apply(e, "workbuddy", 13).unwrap();
        assert_eq!(m.running(14).len(), 2);
        assert_eq!(m.running(14)[0].event.task_id, "other");
        assert_eq!(m.running(14)[1].status(), Status::Waiting);
        assert_eq!(m.current(14).unwrap().status(), Status::Waiting);
        m.selected = Some("workbuddy:new-turn".into());
        assert_eq!(m.current(14).unwrap().status(), Status::Waiting);
        let mut update = m.running(14)[0].event.clone();
        update.seq += 1;
        update.status = Status::Waiting;
        m.apply(update, "workbuddy", 15).unwrap();
        assert_eq!(m.running(15)[0].event.task_id, "other");
        assert!(m.running(2000).is_empty());
    }
    #[test]
    fn terminal_task_revives_on_later_nonterminal_event() {
        let mut m = Model::default();
        // 真实场景：上一轮的 completed 记录延迟落盘，排在新一轮 user 之后，
        // 同一 task 先被判完成；随后的工具活动必须能把任务复活。
        assert!(m.apply(ev(1, Status::Working), "workbuddy", 10).unwrap());
        assert!(m.apply(ev(2, Status::Done), "workbuddy", 11).unwrap());
        assert!(m.apply(ev(3, Status::Working), "workbuddy", 12).unwrap());
        assert_eq!(m.current(13).unwrap().status(), Status::Working);
        // 终态改终态仍然拒绝
        assert!(m.apply(ev(4, Status::Error), "workbuddy", 13).unwrap());
        assert!(m.apply(ev(5, Status::Done), "workbuddy", 14).is_err());
    }
    #[test]
    fn ordered_idempotent_and_terminal() {
        let mut m = Model::default();
        assert!(m.apply(ev(2, Status::Working), "agent", 10).unwrap());
        assert!(!m.apply(ev(1, Status::Done), "agent", 11).unwrap());
        assert!(m.apply(ev(3, Status::Done), "agent", 12).unwrap());
        // 完成态之后回到工作态必须复活（补写记录场景），再报完成则拒绝。
        assert!(m.apply(ev(4, Status::Working), "agent", 13).unwrap());
        assert!(m.apply(ev(5, Status::Done), "agent", 14).unwrap());
        assert!(m.apply(ev(6, Status::Stopped), "agent", 15).is_err());
        assert_eq!(m.current(16).unwrap().status(), Status::Done);
    }
    #[test]
    fn stale_never_becomes_success_and_unknown_progress_stays_unknown() {
        let mut m = Model::default();
        m.apply(ev(1, Status::Working), "agent", 10).unwrap();
        let t = m.current(1000).unwrap();
        assert_eq!(t.status(), Status::Unknown);
        assert_eq!(t.event.progress, None);
        m.apply(ev(2, Status::Done), "agent", 1001).unwrap();
        assert_eq!(m.current(3000).unwrap().status(), Status::Done);
    }
    #[test]
    fn sources_are_isolated_and_demo_is_not_persisted() {
        let mut m = Model::default();
        m.settings.follow_enabled = false;
        m.apply(ev(1, Status::Working), "workbuddy", 10).unwrap();
        assert!(m.current(11).is_none());
        m.apply(ev(1, Status::Waiting), "agent", 11).unwrap();
        m.demo = Some(Status::Done);
        assert_eq!(m.current(12).unwrap().source, "demo");
        let saved = serde_json::to_string(&m).unwrap();
        let restored: Model = serde_json::from_str(&saved).unwrap();
        assert_eq!(restored.current(12).unwrap().status(), Status::Waiting);
    }
}
