use crate::model::{Model, ProgressSource, Status, Task, TASKS_PER_PAGE};
use crate::sprite;
use serde_json::{json, Value};

fn rect(e: &mut Vec<Value>, x: i32, y: i32, w: i32, h: i32, fill: Option<u8>) {
    let mut v = json!({"type":"rect","x":x,"y":y,"w":w,"h":h});
    if let Some(f) = fill {
        v["fill"] = json!(f);
    }
    e.push(v);
}
/// 圆角矩形（黑底白字徽章用）。
fn rect_r(e: &mut Vec<Value>, x: i32, y: i32, w: i32, h: i32, r: i32, fill: Option<u8>) {
    let mut v = json!({"type":"rect","x":x,"y":y,"w":w,"h":h,"radius":r});
    if let Some(f) = fill {
        v["fill"] = json!(f);
    }
    e.push(v);
}
/// 黑底白字徽章里的文字（白色粗体）。
fn text_white(e: &mut Vec<Value>, s: &str, x: i32, y: i32, size: i32, width: i32) {
    e.push(json!({"type":"text","content":s,"x":x,"y":y,"size":size,"width_limit":width,"weight":"bold","color":255}));
}
fn text(e: &mut Vec<Value>, s: &str, x: i32, y: i32, size: i32, width: i32) {
    e.push(json!({"type":"text","content":s,"x":x,"y":y,"size":size,"width_limit":width,"weight":if size>=24 {"bold"} else {"regular"}}));
}
fn text_right(e: &mut Vec<Value>, s: &str, x: i32, y: i32, size: i32, width: i32) {
    e.push(json!({"type":"text","content":s,"x":x,"y":y,"size":size,"width_limit":width,"align":"right","weight":"regular"}));
}
fn title_lines(value: &str, width: usize, max: usize) -> Vec<String> {
    let mut lines = vec![String::new()];
    let mut used = 0.0;
    for ch in value.chars() {
        // 本机渲染用微软雅黑：ASCII 实宽约为 CJK 的 0.3 倍，按 1 估会把单词拦腰截断
        let w = if ch.is_ascii() { 0.6 } else { 2.0 };
        if used + w > width as f32 {
            if lines.len() == max {
                let last = lines.last_mut().unwrap();
                last.pop();
                last.push('…');
                break;
            }
            lines.push(String::new());
            used = 0.0;
        }
        lines.last_mut().unwrap().push(ch);
        used += w;
    }
    lines
}
/// 千分位格式化：2023.15 → "2,023.15"（与设置页显示一致）。
/// 左下角的上下文占用：`上下文 40% · 剩 181K`。
/// 百分比取整、剩余按 K 取整——墨水屏上小数点没有意义，
/// 而「还剩多少」比「已用多少」更能决定该不该新开会话。
fn ctx_label(model: &Model) -> Option<String> {
    let (used, size) = match (model.ctx_used, model.ctx_size) {
        (Some(u), Some(s)) if s > 0 => (u, s),
        _ => return None,
    };
    let pct = ((used as f64 / size as f64) * 100.0).round() as u32;
    let left = size.saturating_sub(used);
    let left = if left >= 1000 {
        format!("剩 {}K", (left as f64 / 1000.0).round() as u32)
    } else {
        "剩 <1K".to_string()
    };
    Some(format!("上下文 {pct}% · {left}"))
}

pub fn draw(model: &Model, now: i64, frame: usize) -> Value {
    // 总结视图优先：用户按了上下键就是明确要看这一页，别再混进任务状态。
    // 条件同时校验「仍是完成态、文本还在」——期间任务被新轮次顶掉时要退回主界面。
    if let (Some(page), Some(t)) = (model.summary_page, model.current(now)) {
        if model.settings.summary_enabled
            && t.status() == Status::Done
            && !t.event.summary.is_empty()
        {
            return summary(&t, page).0;
        }
    }
    let t = model.current(now);
    let state = t.as_ref().map(|t| t.status());
    let working = state == Some(Status::Working);
    let animate = working && model.settings.animation;
    let phase = if animate { frame % 6 } else { 0 };
    let mut e = vec![];
    // 需要人处理时，界面让位给「回电脑操作」指引，不再做任务/选项导航。
    let needs_action = state == Some(Status::Waiting);
    text(&mut e, "ZECTRIX / WORKBUDDY PET", 16, 19, 12, 260);
    // 右上角实时/演示徽章：圆角黑底白字、加大、靠右上对齐；文字在徽章内居中
    //（水平：2 字 28px 在 58 宽内居中；垂直：14 号字帽高约 10px，基线取 8+(22+10)/2=24）。
    rect_r(&mut e, 326, 8, 58, 22, 7, Some(0));
    text_white(
        &mut e,
        if model.demo.is_some() {
            "演示"
        } else {
            "实时"
        },
        341,
        24,
        14,
        30,
    );
    text(&mut e, "AI 工作搭子", 16, 59, 28, 340);
    rect(&mut e, 16, 82, 368, 0, None);
    let active = model.running(now);
    if active.len() > 1 && model.demo.is_none() {
        let pages = active.len().div_ceil(TASKS_PER_PAGE);
        let page = model
            .selected
            .as_ref()
            .and_then(|key| active.iter().position(|t| &t.key() == key))
            .unwrap_or(0)
            / TASKS_PER_PAGE;
        let heading = if pages > 1 {
            format!("{} 个任务 · {}/{}", active.len(), page + 1, pages)
        } else {
            format!("{} 个任务", active.len())
        };
        text(&mut e, &heading, 20, 112, 20, 224);
        let row_height = match active.len() {
            2 => 62,
            3 => 45,
            _ => 36,
        };
        for (i, task) in active
            .iter()
            .skip(page * TASKS_PER_PAGE)
            .take(TASKS_PER_PAGE)
            .enumerate()
        {
            let y = 143 + i as i32 * row_height;
            let title = title_lines(&task.event.title, 24, 1).join("");
            text(&mut e, &title, 20, y, 17, 220);
            let status = match task.event.progress {
                Some(p) => format!("{} · {}%", task.label(), p),
                // 无百分比时用最近活动描述补位，让每行都能回答「在干什么」。
                None => {
                    if task.status() == Status::Working && !task.event.detail.is_empty() {
                        format!("{} · {}", task.label(), task.event.detail)
                    } else {
                        task.label().into()
                    }
                }
            };
            text(&mut e, &status, 20, y + 16, 11, 220);
        }
    } else {
        let headline = t.as_ref().map(|t| t.label()).unwrap_or("等待任务");
        text(&mut e, headline, 20, 116, 22, 224);
        let title = t.as_ref().map(|t| t.event.title.as_str()).unwrap_or(
            if model.settings.follow_enabled {
                "WorkBuddy 跟随已开启"
            } else {
                "本机 WorkBuddy 跟随已关闭"
            },
        );
        for (line, title) in title_lines(title, 24, 3).iter().enumerate() {
            text(&mut e, title, 20, 146 + line as i32 * 21, 18, 220);
        }
        // AskUserQuestion 上屏：只显示题干 + 「回电脑处理」提示，不列选项。
        // 设备无法点击电脑上的选项，列出选项只会让人误以为能在设备上选。
        let ask = t
            .as_ref()
            .filter(|t| t.status() == Status::Waiting)
            .map(|t| t.event.detail.clone())
            .filter(|d| {
                !d.is_empty() && !d.starts_with("WorkBuddy 等待") && !d.starts_with("权限审批")
            });
        // 完成页先说结论：把总结的第一行作为「本轮任务已结束」的替代，
        // 不用按键也能看到 AI 最后说了什么；要看全文再按下键翻页。
        let summary_head = t
            .as_ref()
            .filter(|t| {
                model.settings.summary_enabled
                    && t.status() == Status::Done
                    && !t.event.summary.is_empty()
            })
            .and_then(|t| t.event.summary.lines().next())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let detail = t
            .as_ref()
            .map(|t| {
                if t.stale && !t.event.status.terminal() {
                    "一段时间未收到更新".to_string()
                } else if t.event.detail.starts_with("权限审批") {
                    t.event.detail.clone()
                } else if t.status() == Status::Waiting {
                    match &ask {
                        Some(q) => q.clone(),
                        None => "请回到 WorkBuddy 确认".to_string(),
                    }
                } else if t.status() == Status::Done {
                    match &summary_head {
                        Some(head) => title_lines(head, 30, 1).join(""),
                        None => "本轮任务已结束".to_string(),
                    }
                } else if t.status() == Status::Working {
                    // 活动描述留给下方进度区显示，这里避免重复。
                    "持续跟踪任务执行".to_string()
                } else {
                    t.event.detail.clone()
                }
            })
            .unwrap_or(if model.settings.follow_enabled {
                "在本机 WorkBuddy 中开始一个任务".to_string()
            } else {
                "请在网页开启跟随或接入 Agent".to_string()
            });
        text(
            &mut e,
            &title_lines(&detail, 30, 1).join(""),
            20,
            213,
            13,
            220,
        );
        // 题干放不下时的第二行（指引框上方，13 号字基线 230 不碰到框顶 240）。
        if let Some(q) = &ask {
            let lines = title_lines(q, 30, 2);
            if lines.len() > 1 {
                text(&mut e, &lines[1], 20, 230, 13, 220);
            }
        }
        // 需要人处理时：圆角黑底白字徽章，一句话说清去哪确认；整体靠左对齐。
        if needs_action {
            rect_r(&mut e, 18, 238, 122, 24, 7, Some(0));
            text_white(&mut e, "请回到电脑确认", 30, 255, 14, 106);
        }
        // 需要人处理时不画进度区：那条指引才是这一刻唯一要说的事。
        let progress = t.as_ref().filter(|_| !needs_action).and_then(|t| {
            t.event
                .progress
                .map(|p| (p, t.event.progress_source, t.event.progress_steps))
        });
        match progress {
            Some((p, source, steps)) => {
                rect(&mut e, 20, 228, 212, 13, None);
                if p > 0 {
                    rect(
                        &mut e,
                        23,
                        231,
                        (206u32 * p as u32 / 100) as i32,
                        7,
                        Some(0),
                    );
                }
                // 来源不同，措辞必须不同：清单完成度不是 Agent 上报的进度。
                let label = match (source, steps) {
                    (ProgressSource::Todo, Some((done, total))) => {
                        format!("任务清单 {done}/{total} · {p}%")
                    }
                    (ProgressSource::Todo, None) => format!("任务清单完成度 {p}%"),
                    (ProgressSource::Agent, _) => format!("Agent 上报进度 {p}%"),
                };
                text(&mut e, &label, 20, 258, 12, 218);
            }
            // 没有可核对的百分比时，进度条区域改为文字描述当前实施状态
            //（来自最近一次工具活动，只取工具名，不含任何参数或正文）。
            None if working => {
                let activity = t
                    .as_ref()
                    .map(|t| t.event.detail.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("处理中，暂无活动明细");
                text(
                    &mut e,
                    &title_lines(&format!("当前实施状态：{activity}"), 34, 1).join(""),
                    20,
                    236,
                    14,
                    300,
                );
            }
            None => {}
        }
    }
    // 图鉴造型：30×40 点阵、4 倍像素，落在右侧 120×160 的区域。
    // 形象、服装与道具由 sprite 按 skin 画，这里只决定位置与动画拍。
    let skin = sprite::Skin::parse(&model.settings.pet_skin).unwrap_or_default();
    sprite::draw(&mut e, 254, 98, 4, skin, state, phase);

    rect(&mut e, 16, 271, 368, 0, None);
    // 左下角：当前展示会话的上下文占用（读 WorkBuddy 的 session_usage.used/size）。
    // 选它的原因：本地精确可读、实时更新，且直接决定「还能聊多久」——
    // 云端积分余额本地拿不到，手填快照会失真，已下线。
    let source = match t
        .as_ref()
        .map(|t| (t.source.as_str(), t.event.agent.as_str()))
    {
        Some(("demo", _)) => "演示数据".to_string(),
        Some(("workbuddy", _)) => match ctx_label(model) {
            Some(label) => label,
            None => "WorkBuddy 本机".to_string(),
        },
        Some((_, agent)) if !agent.is_empty() => agent.to_string(),
        _ => "Agent 上报".to_string(),
    };
    text(&mut e, &source, 16, 285, 11, 143);
    // 右下角：等待确认时保持回电脑指引；其余显示随任务变动轮换的花絮短语
    // （WorkBuddy UI 的等待文案是前端随机的，磁盘拿不到，这里是同风格同步换）。
    let quip = if model.quip.is_empty() {
        "跟随中"
    } else {
        model.quip.as_str()
    };
    // 完成态且这一轮有总结时，右下角改成明确的入口：不说「按哪个键」，
    // 用户根本不知道屏幕还能往下翻（设备只有三个键，没有提示就等于没有）。
    let can_summary = model.settings.summary_enabled
        && model.summary_page.is_none()
        && t.as_ref()
            .is_some_and(|t| t.status() == Status::Done && !t.event.summary.is_empty());
    text_right(
        &mut e,
        if needs_action {
            "设备仅作提示"
        } else if can_summary {
            "↓ 看总结"
        } else {
            quip
        },
        170,
        285,
        11,
        214,
    );
    json!({"canvas":{"w":400,"h":300},"elements":e})
}

/// 总结一页放几行、每行几个汉字。
///
/// 15 号字在 360px 宽里约 23 个汉字；正文区从 y=104 到 y=246，行距 21，
/// 正好 7 行。算多一行会压到底部状态栏上，算少一行则要多翻一页。
const SUMMARY_LINE_CHARS: usize = 23;
const SUMMARY_LINES: usize = 7;
/// 翻页上限：再多就不是「总结」而是原文了，末尾直接截断。
const SUMMARY_MAX_PAGES: usize = 8;

/// 把总结文本折行、分页。返回每页的行，页与页之间不重叠。
///
/// 折行复用 `title_lines` 的宽度估算（汉字按 2、ASCII 按 0.6），和设备上的
/// 实际排版一致；段落之间不额外留空行——墨水屏上省下来的每一行都是内容。
pub fn summary_pages(text: &str) -> Vec<Vec<String>> {
    let mut lines: Vec<String> = Vec::new();
    for para in text.lines() {
        let p = para.trim();
        if p.is_empty() {
            continue;
        }
        lines.extend(title_lines(p, SUMMARY_LINE_CHARS * 2, usize::MAX));
    }
    if lines.is_empty() {
        return vec![Vec::new()];
    }
    let mut pages: Vec<Vec<String>> = lines
        .chunks(SUMMARY_LINES)
        .map(|c| c.to_vec())
        .collect();
    if pages.len() > SUMMARY_MAX_PAGES {
        pages.truncate(SUMMARY_MAX_PAGES);
        if let Some(last) = pages.last_mut().and_then(|p| p.last_mut()) {
            if !last.ends_with('…') {
                last.push('…');
            }
        }
    }
    pages
}

/// 完成页的「翻页看总结」：标题 + 正文 + 页码。
///
/// 一页只做三件事：这是哪个任务（标题）、AI 说了什么（正文）、现在在哪一页
/// （页脚）。不画宠物——正文需要整块宽度，形象在这里只会挤掉两行字。
pub fn summary(task: &Task, page: usize) -> (Value, usize) {
    let pages = summary_pages(&task.event.summary);
    let total = pages.len();
    let idx = page.min(total.saturating_sub(1));
    let body = pages.get(idx).cloned().unwrap_or_default();
    let mut e = vec![];

    text(&mut e, "ZECTRIX / 任务总结", 16, 19, 12, 260);
    rect_r(&mut e, 326, 8, 58, 22, 7, Some(0));
    text_white(&mut e, "总结", 341, 24, 14, 30);

    // 标题最多两行：再长就只剩不下正文，而总结页的重点是内容不是名字。
    let title = title_lines(&task.event.title, 24, 2);
    let title_rows = title.len() as i32;
    for (i, line) in title.iter().enumerate() {
        text(&mut e, line, 20, 52 + i as i32 * 20, 17, 340);
    }
    rect(&mut e, 16, 62 + title_rows * 20, 368, 0, None);

    // 标题占两行时正文从 104 起，占一行时多给一行空间。
    let top = if title_rows > 1 { 104 } else { 100 };
    let rows = if title_rows > 1 { SUMMARY_LINES } else { SUMMARY_LINES + 1 };
    for (i, line) in body.iter().take(rows).enumerate() {
        text(&mut e, line, 20, top + i as i32 * 21, 15, 360);
    }
    if body.is_empty() {
        text(&mut e, "这一轮没有可显示的文本", 20, top, 15, 360);
    }

    rect(&mut e, 16, 271, 368, 0, None);
    text(&mut e, &format!("第 {}/{} 页", idx + 1, total), 16, 285, 11, 120);
    text_right(&mut e, "↑↓ 翻页 · 确认返回", 170, 285, 11, 214);
    (json!({"canvas":{"w":400,"h":300},"elements":e}), total)
}

/// 语音投递确认页：识别文本 + 目标会话 + 还能等几秒。
///
/// 这一页要回答三个问题，所以版式按「结论 → 内容 → 操作」竖着排：
/// 投给谁（标题）、说了什么（正文）、现在能做什么（底部）。
pub fn voice(draft: &crate::voice::Draft, now: i64) -> Value {
    let mut e = vec![];
    text(&mut e, "ZECTRIX / 语音投递", 16, 19, 12, 260);
    rect_r(&mut e, 326, 8, 58, 22, 7, Some(0));
    text_white(&mut e, "语音", 341, 24, 14, 30);

    // 目标会话：没有会话时也要说清楚「会粘贴到哪儿」，不能让人以为丢失了。
    let target = match draft.target() {
        Some(t) => format!("→ {}", t.title),
        None => "→ 当前窗口".to_string(),
    };
    for (i, line) in title_lines(&target, 26, 1).iter().enumerate() {
        text(&mut e, line, 16, 59 + i as i32 * 30, 24, 340);
    }
    rect(&mut e, 16, 84, 368, 0, None);

    let body = if draft.text.trim().is_empty() {
        "（没听清，再说一次）".to_string()
    } else {
        draft.text.trim().to_string()
    };
    // 短句放大、长句缩小：一句话占半屏时 15 号字太小，一个字都看不清；
    // 而长句必须能整段塞进来，所以按行数反过来选字号。
    let mut lines = title_lines(&body, 48, 6);
    let (size, lh) = if lines.len() <= 2 {
        lines = title_lines(&body, 40, 5);
        (18, 25)
    } else {
        (15, 19)
    };
    for (i, line) in lines.iter().enumerate() {
        text(&mut e, line, 16, 106 + i as i32 * lh, size, 360);
    }
    // 小字跟在正文后面，别钉死在底部：只说一句话时中间留一大片空白很难看。
    let foot_y = (106 + lines.len() as i32 * lh + 6).min(228);
    // 第二行小字平时报「录了几秒、只粘贴不发送」；已粘贴后换成「还能改字」，
    // 投递失败后换成失败原因——让人知道该再按一次还是去电脑上看看窗口。
    let foot = draft.hint.clone().unwrap_or_else(|| {
        if draft.pasted {
            "已粘贴，可在电脑上改字".to_string()
        } else {
            format!("{:.1} 秒录音 · 先粘贴，不发送", draft.seconds)
        }
    });
    text(&mut e, &foot, 16, foot_y, 11, 330);

    // 底部操作条：黑底白字，和主界面的「回电脑确认」一致，读一眼就知道下一步。
    rect_r(&mut e, 16, 240, 368, 30, 6, Some(0));
    // 已粘贴时只有一件事可做：发送。这时还提示「换会话」会把人引到错的方向
    //（文字已经在那个会话的输入框里了，换目标只会让它留在原地）。
    let hint = if draft.pasted {
        "确认键 发送这条消息".to_string()
    } else if draft.targets.len() > 1 {
        format!(
            "上下键 换会话（{}/{}） · 确认键 粘贴",
            draft.index + 1,
            draft.targets.len()
        )
    } else {
        "确认键 粘贴到输入框".to_string()
    };
    text_white(&mut e, &hint, 26, 260, 13, 300);

    let left = (draft.expires_at - now).max(0);
    text(
        &mut e,
        &if draft.pasted {
            format!("{left} 秒内可发送")
        } else {
            format!("{left} 秒后丢弃")
        },
        16,
        290,
        11,
        200,
    );
    text_right(&mut e, "语音由本机转写", 200, 290, 11, 184);

    json!({"canvas":{"w":400,"h":300},"elements":e})
}
/// 语音流程中的过渡页（「识别中」「没听清」「投递失败」）。
///
/// 只放一句话 + 一行解释：这些状态都是一两秒就过去的，不需要更多信息，
/// 多写反而让人以为要等他操作。
pub fn voice_note(title: &str, detail: &str, dots: usize) -> Value {
    let mut e = vec![];
    text(&mut e, "ZECTRIX / 语音投递", 16, 19, 12, 260);
    text(&mut e, title, 16, 104, 26, 368);
    for (i, line) in title_lines(detail, 46, 3).iter().enumerate() {
        text(&mut e, line, 16, 150 + i as i32 * 20, 15, 360);
    }
    // 三个点的进度节奏：不给具体百分比，因为转写耗时会随句子长短变。
    let waiting = "·".repeat(dots.min(3)) + &" ".repeat(3 - dots.min(3));
    text(&mut e, &waiting, 16, 240, 18, 200);
    text_right(&mut e, "语音由本机转写", 200, 290, 11, 184);
    json!({"canvas":{"w":400,"h":300},"elements":e})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn voice_pages_stay_inside_the_canvas() {
        let targets = vec![
            crate::voice::Target {
                session: "a".into(),
                title: "基础固件编译".into(),
            },
            crate::voice::Target {
                session: "b".into(),
                title: "写周报".into(),
            },
        ];
        let d = crate::voice::Draft::new(
            "把今天的会议纪要整理成三条要点，发到项目群里。".into(),
            3.2,
            targets,
            100,
        );
        let scenes = [
            voice(&d, 100),
            voice(&d, 130), // 过期后剩 0 秒，不能出现负数
            voice_note("正在识别…", "松手后在本机转写，音频不上传云端", 2),
            voice_note("转写失败", "很长的错误信息也要能折行放进版心，不能顶到边框外面去", 0),
        ];
        for v in scenes {
            let e = v["elements"].as_array().unwrap();
            assert!(e.len() <= 200);
            for el in e {
                assert!(el["x"].as_i64().unwrap() >= 0);
                assert!(el["y"].as_i64().unwrap() < 300);
            }
        }
    }

    #[test]
    fn voice_draft_cycles_between_sessions_only_when_there_are_two() {
        let t = |s: &str| crate::voice::Target {
            session: s.into(),
            title: s.into(),
        };
        let mut one = crate::voice::Draft::new("x".into(), 1.0, vec![t("a")], 0);
        one.cycle(1);
        one.cycle(-1);
        assert_eq!(one.index, 0, "只有一个目标时按键不该改变选择");

        let mut two = crate::voice::Draft::new("x".into(), 1.0, vec![t("a"), t("b")], 0);
        two.cycle(1);
        assert_eq!(two.target().unwrap().session, "b");
        two.cycle(1);
        assert_eq!(two.target().unwrap().session, "a", "到末尾要绕回第一个");
        two.cycle(-1);
        assert_eq!(two.target().unwrap().session, "b", "往前也要绕");
    }

    #[test]
    fn pasted_draft_stops_cycling_targets() {
        let t = |s: &str| crate::voice::Target {
            session: s.into(),
            title: s.into(),
        };
        let mut d = crate::voice::Draft::new("x".into(), 1.0, vec![t("a"), t("b")], 0);
        d.pasted = true;
        d.cycle(1);
        assert_eq!(
            d.target().unwrap().session,
            "a",
            "已经粘贴后不能再换目标——文字留在原会话的输入框里"
        );
    }

    #[test]
    fn summary_pages_wrap_and_respect_the_reading_area() {
        // 一段远超一页的文本：要分成多页、每页不超行数、每行不超宽。
        let text = "第一段结论。\n".to_string() + &"很长的说明文字，用来填满好几页。".repeat(40);
        let pages = summary_pages(&text);
        assert!(pages.len() > 1, "长文本必须分页");
        assert!(pages.len() <= SUMMARY_MAX_PAGES);
        for page in &pages {
            assert!(!page.is_empty());
            assert!(page.len() <= SUMMARY_LINES + 1);
            for line in page {
                let w: f32 = line
                    .chars()
                    .map(|c| if c.is_ascii() { 0.6 } else { 2.0 })
                    .sum();
                assert!(w <= (SUMMARY_LINE_CHARS * 2) as f32 + 0.1, "行超宽：{line}");
            }
        }
        // 短文本只有一页，空文本也返回一页（渲染端据此显示占位文案）。
        assert_eq!(summary_pages("就一句话").len(), 1);
        assert_eq!(summary_pages("").len(), 1);
    }

    #[test]
    fn summary_scene_stays_inside_the_canvas() {
        let task = Task {
            event: crate::model::Event {
                task_id: "t".into(),
                thread_id: None,
                seq: 1,
                status: Status::Done,
                title: "把项目文档补齐并发布".into(),
                detail: String::new(),
                progress: None,
                progress_source: crate::model::ProgressSource::Agent,
                progress_steps: None,
                agent: "WorkBuddy".into(),
                summary: "结论：三处文档已更新。\n".to_string() + &"细节。".repeat(200),
            },
            source: "workbuddy".into(),
            updated_at: 1_700_000_000,
            stale: false,
        };
        let (scene, total) = summary(&task, 0);
        assert!(total > 1);
        let e = scene["elements"].as_array().unwrap();
        assert!(e.len() <= 80);
        for el in e {
            assert!(el["x"].as_i64().unwrap() >= 0);
            assert!(el["y"].as_i64().unwrap() < 300);
        }
    }

    #[test]
    fn fixed_rows_survive_heartbeats_selection_and_time() {
        for count in [2usize, 3, 4, 5, 9, 10] {
            let mut m = Model::default();
            for i in 0..count {
                m.apply(
                    crate::model::Event {
                        task_id: format!("task-{i:03}"),
                        thread_id: None,
                        seq: 1,
                        status: Status::Working,
                        title: format!("任务名称{i}"),
                        detail: String::new(),
                        progress: None,
                        progress_source: crate::model::ProgressSource::Agent,
                        progress_steps: None,
                        agent: "Test".into(),
                        summary: String::new(),
                    },
                    "agent",
                    1,
                )
                .unwrap();
            }
            // apply 会轮换右下角花絮短语；本测试只关心行序，先冻结它。
            m.quip = "测试花絮".into();
            let baseline = draw(&m, 1, 0);
            assert_eq!(baseline, draw(&m, 60, 0), "no automatic rotation");
            let mut heartbeat = m.running(1)[0].event.clone();
            heartbeat.seq = 2;
            m.apply(heartbeat, "agent", 61).unwrap();
            m.quip = "测试花絮".into();
            assert_eq!(baseline, draw(&m, 61, 0), "heartbeat must not reorder rows");
            let pages = count.div_ceil(TASKS_PER_PAGE);
            for page in 0..pages {
                let v = draw(&m, 62, 0);
                let e = v["elements"].as_array().unwrap();
                let titles: Vec<_> = e
                    .iter()
                    .filter(|e| {
                        e["content"]
                            .as_str()
                            .is_some_and(|s| s.starts_with("任务名称"))
                    })
                    .collect();
                assert_eq!(
                    titles.len(),
                    (count - page * TASKS_PER_PAGE).min(TASKS_PER_PAGE)
                );
                for (row, title) in titles.iter().enumerate() {
                    assert_eq!(
                        title["content"],
                        format!("任务名称{}", page * TASKS_PER_PAGE + row)
                    );
                }
                assert!(titles
                    .windows(2)
                    .all(|w| w[0]["y"].as_i64() < w[1]["y"].as_i64()));
                assert!(!v.to_string().contains('%'));
                assert!(e.len() <= 200);
                if pages > 1 {
                    m.cycle(1, 62);
                }
            }
            assert_eq!(baseline, draw(&m, 62, 0), "page navigation wraps");
            m.selected = Some(m.running(62)[1].key());
            if count <= TASKS_PER_PAGE {
                assert_eq!(
                    baseline,
                    draw(&m, 62, 0),
                    "selection keeps all rows visible"
                );
            } else {
                m.selected = None;
                m.cycle(-1, 62);
                assert!(draw(&m, 62, 0)
                    .to_string()
                    .contains(&format!("{}/{}", pages, pages)));
            }
        }
    }
    #[test]
    fn no_progress_shows_activity_text_instead_of_bar() {
        let mut m = Model::default();
        m.apply(
            crate::model::Event {
                task_id: "task-1".into(),
                thread_id: None,
                seq: 1,
                status: Status::Working,
                title: "接入宠物".into(),
                detail: "正在执行命令".into(),
                progress: None,
                progress_source: crate::model::ProgressSource::Todo,
                progress_steps: None,
                agent: "WorkBuddy".into(),
                summary: String::new(),
            },
            "workbuddy",
            1,
        )
        .unwrap();
        let v = draw(&m, 2, 0);
        let s = v.to_string();
        assert!(
            s.contains("当前实施状态：正在执行命令"),
            "进度区应显示活动描述"
        );
        assert!(s.contains("持续跟踪任务执行"), "detail 行不应与进度区重复");
        assert!(!s.contains('%'), "无清单时不得出现百分比");

        // 有清单时仍是进度条 + 可核对分母，且不再出现状态文字。
        let mut ev = m.current(2).unwrap().event;
        ev.seq = 2;
        ev.progress = Some(43);
        ev.progress_steps = Some((3, 7));
        m.apply(ev, "workbuddy", 2).unwrap();
        let s = draw(&m, 3, 0).to_string();
        assert!(s.contains("任务清单 3/7 · 43%"));
        assert!(!s.contains("当前实施状态"));
    }

    #[test]
    fn bottom_bar_shows_credits_and_quip() {
        let mut m = Model::default();
        m.apply(
            crate::model::Event {
                task_id: "task-1".into(),
                thread_id: None,
                seq: 1,
                status: Status::Working,
                title: "接入宠物".into(),
                detail: String::new(),
                progress: None,
                progress_source: crate::model::ProgressSource::Todo,
                progress_steps: None,
                agent: "WorkBuddy".into(),
                summary: String::new(),
            },
            "workbuddy",
            1,
        )
        .unwrap();
        // 没有上下文数据时保持原标签；花絮短语随 apply 已换好、且来自短语池。
        let s = draw(&m, 2, 0).to_string();
        assert!(s.contains("WorkBuddy 本机"), "无上下文数据回退原标签");
        assert!(!s.contains("上下选任务"), "按键提示已下线");
        assert!(
            crate::model::quip_pool().contains(&m.quip.as_str()),
            "花絮必须来自短语池"
        );

        // 有上下文数据时左下角换成占用百分比与剩余，右下角显示花絮。
        m.ctx_used = Some(119222);
        m.ctx_size = Some(300000);
        let s = draw(&m, 2, 0).to_string();
        assert!(
            s.contains("上下文 40% · 剩 181K"),
            "左下角应显示上下文占用：{s}"
        );
        assert!(s.contains(&m.quip), "右下角应显示花絮短语");
    }

    #[test]
    fn ctx_label_shows_percent_and_headroom() {
        let mut m = Model {
            ctx_used: Some(119222),
            ctx_size: Some(300000),
            ..Model::default()
        };
        assert_eq!(ctx_label(&m).as_deref(), Some("上下文 40% · 剩 181K"));
        m.ctx_used = Some(299600);
        assert_eq!(ctx_label(&m).as_deref(), Some("上下文 100% · 剩 <1K"));
        m.ctx_size = None;
        assert_eq!(ctx_label(&m), None, "没有窗口大小就不编造百分比");
    }

    #[test]
    fn waiting_shows_question_and_call_to_action() {
        let mut m = Model::default();
        m.apply(
            crate::model::Event {
                task_id: "task-1".into(),
                thread_id: None,
                seq: 1,
                status: Status::Waiting,
                title: "接入宠物".into(),
                detail: "采用哪种推送策略？".into(),
                progress: None,
                progress_source: crate::model::ProgressSource::Todo,
                progress_steps: None,
                agent: "WorkBuddy".into(),
                summary: String::new(),
            },
            "workbuddy",
            1,
        )
        .unwrap();
        let v = draw(&m, 2, 0).to_string();
        assert!(v.contains("采用哪种推送策略？"), "题干应上屏");
        assert!(v.contains("请回到电脑确认"), "要有回电脑的指引");
        assert!(v.contains("设备仅作提示"), "底部提示改为仅提示");
        assert!(!v.contains("【"), "不再渲染选项高亮");
        assert!(!v.contains("上下选选项"), "不再提示用上下键选选项");
    }

    #[test]
    fn layouts_obey_hub_limits() {
        let mut m = Model::default();
        for s in [
            Status::Working,
            Status::Waiting,
            Status::Done,
            Status::Error,
            Status::Stopped,
            Status::Unknown,
        ] {
            m.demo = Some(s);
            for frame in 0..6 {
                let v = draw(&m, 100, frame);
                let e = v["elements"].as_array().unwrap();
                assert!(e.len() <= 200);
                for v in e {
                    assert!(v["x"].as_i64().unwrap() >= 0);
                    assert!(v["y"].as_i64().unwrap() < 300);
                }
            }
        }
    }
}
