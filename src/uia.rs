//! 从 WorkBuddy 窗口读当前那条花絮短语。
//!
//! 为什么必须走界面：这条文案是服务端随请求流式下发的，只在渲染进程内存里，
//! 磁盘上没有任何副本。已核查的范围——`app.asar`、`AppData\Roaming\WorkBuddy`
//! 全目录（含 HTTP 缓存、leveldb、Session Storage）、`~/.workbuddy`（会话 JSONL、
//! 日志、workbuddy.db、traces）——用 UTF-8 / UTF-16LE / GBK 三种编码、原样 +
//! gzip + zlib + deflate 四种形态全扫过，一次都没命中。所以守护只读磁盘时
//! 永远拿不到它，只能自己编（即内置短语池）。
//!
//! 界面上它有两副面孔，**不能只认一种**：
//!
//! - 准备阶段（首个 token 之前）：状态行是 `准备中 你在思考大局，我在打磨细节`；
//! - 出 token 之后：同一条文案搬到状态气泡里，和 `已消耗 1.42` 同一组、紧挨着
//!   `不再显示` 按钮，每几秒换一条（`挑剔地筛选每一句`、`稍等，我去把答案捞回来`…）。
//!
//! 关键坑：历史轮次的 `准备中 <旧花絮>` 节点**会留在可访问性树里且永不更新**。
//! 只按「最后一条准备中」取，会一直读到上一轮那句死文字，`set_live_quip` 判定
//! 无变化，设备就永远不刷新（这正是「界面换了十几条、设备一次没动」的原因）。
//! 因此以界面结构定位：**最后一个 `不再显示` 按钮前面那个文本节点**才是当前花絮；
//! 没有按钮时退而取最后一个 `已消耗` 后面的第一条像花絮的文本。
//!
//! 采样时机：**只在确定要更新页面时**（`push_if_changed` 判定这一帧真要上屏
//! 之后）调一次，和这一帧一起推上去；没有独立的定时轮询——界面自己换了十几条
//! 而任务状态没变时，设备不跟着闪屏。

use std::sync::mpsc::{self, Sender};
use std::sync::OnceLock;
use std::time::Duration;

/// 采样超时。UIA 是跨进程 COM 调用，窗口卡顿时不能把守护拖住。
const PROBE_TIMEOUT: Duration = Duration::from_millis(700);

/// 读一次界面上当前的花絮；读不到时返回 None（调用方回退内置短语池）。
pub fn current_quip() -> Option<String> {
    imp::current_quip()
}

/// 可访问性树里节点的类别。只需要区分「文本」「按钮」和「其它」。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    Button,
    Other,
}

/// 从（按出现顺序排好的）节点序列里挑出当前花絮。纯函数，便于单测。
///
/// 依据实测结构：状态气泡是 `… | 已消耗 1.42 <花絮> 不再显示`，
/// 花絮是 `不再显示` 前最后一个文本节点；历史轮次里的 `准备中 <旧花絮>`
/// 节点不参与判断（它会永远停在那里）。
pub fn pick_quip(items: &[(Kind, String)]) -> Option<String> {
    let mut last_text: Option<String> = None;
    let mut by_button: Option<String> = None;
    let mut last_consumed: Option<usize> = None;

    for (index, (kind, name)) in items.iter().enumerate() {
        let name = name.trim();
        match kind {
            Kind::Button if name == "不再显示" => {
                // 气泡结尾：紧挨着它的那个文本就是本轮花絮。
                if let Some(quip) = last_text.as_deref().and_then(implausible_reason_is_none) {
                    by_button = Some(quip);
                }
                last_text = None;
            }
            Kind::Text => {
                if name == "已消耗" {
                    last_consumed = Some(index);
                }
                if !name.is_empty() {
                    last_text = Some(name.to_string());
                }
            }
            _ => {}
        }
    }
    if by_button.is_some() {
        return by_button;
    }
    // 兜底：按钮被用户关掉时，取「已消耗」之后的第一个文本候选。
    let start = last_consumed?;
    items
        .iter()
        .skip(start + 1)
        .take(4)
        .filter(|(kind, _)| *kind == Kind::Text)
        .find_map(|(_, name)| implausible_reason_is_none(name))
}

/// 名字读起来像不像一条花絮；像就返回它，不像返回 None。
///
/// 排掉的是实测中混进来的邻居：消耗数值、进度时长、`深度思考` 之类状态词，
/// 以及气泡被替换成产品引导时的长句（`在「设置 > 个性化」中设置…。`）。
fn implausible_reason_is_none(name: &str) -> Option<String> {
    let text = name.trim();
    let len = text.chars().count();
    if !(4..=30).contains(&len) {
        return None;
    }
    // 引导文案以句号收尾、爱用「」，花絮从不这样。
    if text.ends_with('。') || text.contains('「') {
        return None;
    }
    // 状态词和标签不是花絮；`准备中 …` 那种占位也是。
    const LABELS: [&str; 8] = [
        "已处理",
        "已完成",
        "准备中",
        "深度思考",
        "正在执行命令",
        "已消耗",
        "不再显示",
        "预计消耗",
    ];
    if LABELS.iter().any(|label| text.starts_with(label)) {
        return None;
    }
    // 纯数字/时长/省略号。
    if !text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)) {
        return None;
    }
    Some(text.to_string())
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
        UIA_ButtonControlTypeId, UIA_ControlTypePropertyId, UIA_TextControlTypeId,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowRect, GetWindowTextW, IsWindowVisible,
    };

    /// 待命的工作线程：UIA 必须在初始化过 COM 的线程上调用，且一旦常驻，
    /// 后续采样就只剩一次进程内消息投递的代价。
    static CLIENT: OnceLock<Sender<Sender<Option<String>>>> = OnceLock::new();

    pub fn current_quip() -> Option<String> {
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
                        Some(a) => read_quip(a),
                        None => None,
                    };
                    // 调用方可能已经超时走人，发送失败属正常。
                    let _ = reply.send(value);
                }
            })
            .ok();
        tx
    }

    /// 只把「文本 + 按钮」两类节点按树序摊平，再交给 `pick_quip` 判定。
    /// 用 UIA 的 Or 条件让过滤发生在 WorkBuddy 进程里——整棵 Electron 树有
    /// 400+ 节点，全部拉回本地逐个读属性会慢一个数量级。
    /// UIA 保证 `FindAll` 的结果是树序（深度优先），`pick_quip` 依赖这个顺序。
    fn read_quip(automation: &IUIAutomation) -> Option<String> {
        let root = element_of_workbuddy(automation)?;
        let text_condition = unsafe {
            automation.CreatePropertyCondition(
                UIA_ControlTypePropertyId,
                &VARIANT::from(UIA_TextControlTypeId.0),
            )
        }
        .ok()?;
        let button_condition = unsafe {
            automation.CreatePropertyCondition(
                UIA_ControlTypePropertyId,
                &VARIANT::from(UIA_ButtonControlTypeId.0),
            )
        }
        .ok()?;
        let condition = unsafe { automation.CreateOrCondition(&text_condition, &button_condition) }
            .ok()?;
        let found = unsafe { root.FindAll(TreeScope_Descendants, &condition) }.ok()?;
        let count = unsafe { found.Length() }.ok()?;
        let mut items = Vec::with_capacity(count as usize);
        for index in 0..count {
            let Ok(element) = (unsafe { found.GetElement(index) }) else {
                continue;
            };
            let kind = match unsafe { element.CurrentControlType() } {
                Ok(t) if t == UIA_TextControlTypeId => Kind::Text,
                Ok(t) if t == UIA_ButtonControlTypeId => Kind::Button,
                _ => continue,
            };
            let name = unsafe { element.CurrentName() }
                .map(|b| b.to_string())
                .unwrap_or_default();
            items.push((kind, name));
        }
        pick_quip(&items)
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
    pub fn current_quip() -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{pick_quip, Kind};

    fn text(name: &str) -> (Kind, String) {
        (Kind::Text, name.to_string())
    }
    fn button(name: &str) -> (Kind, String) {
        (Kind::Button, name.to_string())
    }

    #[test]
    fn takes_the_text_right_before_the_dismiss_button() {
        // 实测结构：… 已消耗 1.42 <花絮> 不再显示
        let items = vec![
            text("WorkBuddy"),
            text("预计消耗"),
            text("0.57 ~ 20.09"),
            text("已处理 1m17s"),
            text("正在执行命令"),
            text("|"),
            text("已消耗"),
            text("1.42"),
            text("挑剔地筛选每一句"),
            button("不再显示"),
        ];
        assert_eq!(pick_quip(&items).as_deref(), Some("挑剔地筛选每一句"));
    }

    /// 回归：历史轮次留下的 `准备中 <旧花絮>` 节点永远不更新，
    /// 曾经因为「取最后一条准备中」而让设备一次都刷不出来。
    #[test]
    fn ignores_stale_preparing_lines_from_earlier_turns() {
        let items = vec![
            text("准备中 你在思考大局，我在打磨细节"),
            text("已完成 29m"),
            text("准备中 …"),
            text("已处理 27s"),
            text("已消耗"),
            text("0.9"),
            text("稍等，我去把答案捞回来"),
            button("不再显示"),
        ];
        assert_eq!(pick_quip(&items).as_deref(), Some("稍等，我去把答案捞回来"));
    }

    #[test]
    fn falls_back_to_the_text_after_spending_when_the_button_is_gone() {
        let items = vec![
            text("已消耗"),
            text("1.42"),
            text("别慌，办法总比困难多，多到我得挑一个"),
        ];
        assert_eq!(
            pick_quip(&items).as_deref(),
            Some("别慌，办法总比困难多，多到我得挑一个")
        );
    }

    #[test]
    fn rejects_neighbours_that_are_not_quips() {
        // 气泡被替换成产品引导长句时不能当花絮。
        let items = vec![
            text("已消耗"),
            text("1.42"),
            text("在「设置 > 个性化」中设置希望被怎样称呼，让对话从第一句就更自然。"),
            button("不再显示"),
        ];
        assert_eq!(pick_quip(&items), None);
        // 空文本、纯数值、状态词都不算。
        assert_eq!(pick_quip(&[text("已消耗"), text("1.42"), button("不再显示")]), None);
        assert_eq!(pick_quip(&[]), None);
    }

    /// 真机联通性：窗口不在或读不到时返回 None 也算通过，只验证不 panic、不卡死。
    /// `cargo test -- --ignored` 手动跑。
    #[test]
    #[ignore = "需要 WorkBuddy 正在运行"]
    fn probes_live_window_without_panicking() {
        let _ = super::current_quip();
    }
}
