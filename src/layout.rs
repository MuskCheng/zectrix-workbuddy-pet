use crate::model::{Model, ProgressSource, Status, TASKS_PER_PAGE};
use serde_json::{json, Value};

// Original pixel companion from the earlier AI 工作搭子 prototype, expressed as
// horizontal runs rather than a full-screen bitmap to stay within Hub v1's 200 elements.
const PET: [&str; 16] = [
    "..###......###..",
    "..#..#....#..#..",
    "..#...####...#..",
    ".#............#.",
    "#..............#",
    "#..............#",
    "#..............#",
    "#..............#",
    ".#............#.",
    "..#..........#..",
    "...##########...",
    "....#......#....",
    "....#......#....",
    "...#........#...",
    "..############..",
    "..##........##..",
];
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
// Restore the original raised arms, smiling face and three sparkles. Rasterize
// only these small strokes into merged rectangles supported by Hub v1.
fn cheer(e: &mut Vec<Value>, x: i32, y: i32, p: i32) {
    let mut pixels = std::collections::BTreeSet::new();
    let mut stroke = |points: &[(i32, i32)], width: i32| {
        for segment in points.windows(2) {
            let ((x0, y0), (x1, y1)) = (segment[0], segment[1]);
            let steps = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
            for i in 0..=steps {
                let px = x0 + ((x1 - x0) as f64 * i as f64 / steps as f64).round() as i32;
                let py = y0 + ((y1 - y0) as f64 * i as f64 / steps as f64).round() as i32;
                for dy in 0..width {
                    for dx in 0..width {
                        pixels.insert((py + dy - width / 2, px + dx - width / 2));
                    }
                }
            }
        }
    };
    for ex in [x + 4 * p, x + 10 * p] {
        stroke(
            &[
                (ex, y + 6 * p),
                (ex + p, y + 5 * p),
                (ex + 2 * p, y + 6 * p),
            ],
            2,
        );
    }
    stroke(
        &[
            (x + 6 * p, y + 8 * p),
            (x + 7 * p, y + 9 * p),
            (x + 9 * p, y + 9 * p),
            (x + 10 * p, y + 8 * p),
        ],
        2,
    );
    stroke(&[(x + 3 * p, y + 12 * p), (x - p, y + 9 * p)], 3);
    stroke(&[(x + 13 * p, y + 12 * p), (x + 17 * p, y + 9 * p)], 3);
    for (xx, yy) in [(x - 7, y - 10), (x + 100, y - 11), (x + 115, y + 30)] {
        stroke(&[(xx, yy), (xx + 7, yy + 7)], 2);
        stroke(&[(xx + 7, yy), (xx, yy + 7)], 2);
    }
    let mut spans: Vec<(i32, i32, i32)> = vec![];
    for (py, px) in pixels {
        if let Some((row, start, width)) = spans.last_mut() {
            if *row == py && *start + *width == px {
                *width += 1;
                continue;
            }
        }
        spans.push((py, px, 1));
    }
    let mut blocks: Vec<(i32, i32, i32, i32)> = vec![];
    for (py, px, width) in spans {
        if let Some(block) = blocks
            .iter_mut()
            .rev()
            .find(|b| b.0 == px && b.2 == width && b.1 + b.3 == py)
        {
            block.3 += 1;
        } else {
            blocks.push((px, py, width, 1));
        }
    }
    for (px, py, w, h) in blocks {
        rect(e, px, py, w, h, Some(0));
    }
}
fn text(e: &mut Vec<Value>, s: &str, x: i32, y: i32, size: i32, width: i32) {
    e.push(json!({"type":"text","content":s,"x":x,"y":y,"size":size,"width_limit":width,"weight":if size>=24 {"bold"} else {"regular"}}));
}
/// 小字号但需要强调的文本（大字号默认粗体，小字号需显式指定）。
fn text_bold(e: &mut Vec<Value>, s: &str, x: i32, y: i32, size: i32, width: i32) {
    e.push(json!({"type":"text","content":s,"x":x,"y":y,"size":size,"width_limit":width,"weight":"bold"}));
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
                    "本轮任务已结束".to_string()
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
    let x = 254;
    let y = 103;
    let p = 7;
    let dx = if animate {
        [0, 1, 2, 1, 0, -1][phase]
    } else {
        0
    };
    for (row, line) in PET.iter().enumerate() {
        let bytes = line.as_bytes();
        let mut col = 0;
        while col < bytes.len() {
            if bytes[col] != b'#' {
                col += 1;
                continue;
            }
            let start = col;
            while col < bytes.len() && bytes[col] == b'#' {
                col += 1;
            }
            rect(
                &mut e,
                x + start as i32 * p + if row <= 10 { dx } else { 0 },
                y + row as i32 * p,
                (col - start) as i32 * p - 1,
                p - 1,
                Some(0),
            );
        }
    }
    if state == Some(Status::Done) {
        cheer(&mut e, x, y, p);
    } else {
        for ex in [x + 4 * p + dx, x + 10 * p + dx] {
            if state == Some(Status::Error) {
                rect(&mut e, ex, y + 5 * p, p * 2, 3, Some(0));
                rect(&mut e, ex + p, y + 5 * p, 3, p * 2, Some(0));
            } else {
                rect(
                    &mut e,
                    ex,
                    y + 5 * p,
                    p,
                    if phase == 3 { 2 } else { p * 2 },
                    Some(0),
                );
            }
        }
        rect(
            &mut e,
            x + 7 * p + dx,
            y + 8 * p,
            p * 2,
            if state == Some(Status::Waiting) {
                p * 2
            } else {
                p
            },
            Some(0),
        );
    }
    if working {
        rect(&mut e, x + 2 * p, y + 13 * p, 12 * p, 3 * p, Some(255));
        rect(&mut e, x + 2 * p, y + 13 * p, 12 * p, 3 * p, None);
        for col in 0..4 {
            rect(&mut e, x + (4 + 2 * col) * p, y + 14 * p, p, 1, Some(0));
        }
        rect(
            &mut e,
            285,
            if phase % 2 == 0 { 193 } else { 185 },
            8,
            7,
            Some(0),
        );
        rect(
            &mut e,
            327,
            if phase % 2 == 1 { 193 } else { 185 },
            8,
            7,
            Some(0),
        );
    }
    if state == Some(Status::Done) {
        text(&mut e, "完成！", 274, 246, 18, 105);
    }
    if matches!(state, Some(Status::Waiting | Status::Unknown)) {
        // 分隔线在 y=82、桌宠顶边在 y=103。20 号粗体字高约 15px，基线 101 时
        // 字顶 86、字底 101，上下各留 3px 与 2px，既不压线也不碰桌宠。
        text_bold(&mut e, "?", 354, 101, 20, 26);
    }
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
    text_right(
        &mut e,
        if needs_action {
            "设备仅作提示"
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
#[cfg(test)]
mod tests {
    use super::*;
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
