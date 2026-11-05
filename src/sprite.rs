//! 像素宠物：以「像素喵」为原型的机器猫。
//!
//! 画在一张 30×40 的灰度点阵上，再按色块合并成矩形交给 `layout`。
//! 之所以不用圆角矩形直接画：墨水屏上像素风的识别度靠硬边和有限色阶，
//! 抗锯齿的圆角会让它看起来像 UI 图形而不是角色。
//!
//! 状态只改局部：眼睛（表情）、手臂与道具（行为）、头顶装饰（状态符），
//! 主体（帽子、头、面罩、斗篷、剑、盾）所有状态共用，保证是同一只猫。

use crate::model::Status;
use serde_json::{json, Value};

const W: i32 = 30;
const H: i32 = 40;

const BLACK: u8 = 0;
const DEEP: u8 = 55;
/// 帽子的灰阶比斗篷再深一档：两者同为织物，但在灰度上必须能分开。
const HAT: u8 = 85;
const CLOTH: u8 = 105;
const METAL: u8 = 152;
const LIGHT: u8 = 208;
const WHITE: u8 = 255;

struct Grid {
    cells: Vec<Option<u8>>,
}

impl Grid {
    fn new() -> Self {
        Self {
            cells: vec![None; (W * H) as usize],
        }
    }

    fn set(&mut self, x: i32, y: i32, c: u8) {
        if x < 0 || y < 0 || x >= W || y >= H {
            return;
        }
        self.cells[(y * W + x) as usize] = Some(c);
    }

    fn rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: u8) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.set(x, y, c);
            }
        }
    }

    /// 圆角矩形；圆角用整数距离判定，边缘保持硬像素。
    fn rrect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, r: i32, c: u8) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = if x < x0 + r {
                    x0 + r - x
                } else if x > x1 - r {
                    x - (x1 - r)
                } else {
                    0
                };
                let dy = if y < y0 + r {
                    y0 + r - y
                } else if y > y1 - r {
                    y - (y1 - r)
                } else {
                    0
                };
                if dx * dx + dy * dy <= r * r + r {
                    self.set(x, y, c);
                }
            }
        }
    }

    fn ellipse(&mut self, cx: i32, cy: i32, rx: i32, ry: i32, c: u8) {
        for y in cy - ry..=cy + ry {
            for x in cx - rx..=cx + rx {
                let dx = (x - cx) as f32 / rx as f32;
                let dy = (y - cy) as f32 / ry as f32;
                if dx * dx + dy * dy <= 1.04 {
                    self.set(x, y, c);
                }
            }
        }
    }

    fn circle(&mut self, cx: i32, cy: i32, r: i32, c: u8) {
        for y in cy - r..=cy + r {
            for x in cx - r..=cx + r {
                let (dx, dy) = (x - cx, y - cy);
                if dx * dx + dy * dy <= r * r + r {
                    self.set(x, y, c);
                }
            }
        }
    }

    /// 多边形填充（扫描线）。
    fn poly(&mut self, pts: &[(i32, i32)], c: u8) {
        if pts.len() < 3 {
            return;
        }
        let miny = pts.iter().map(|p| p.1).min().unwrap();
        let maxy = pts.iter().map(|p| p.1).max().unwrap();
        for y in miny..=maxy {
            let mut xs: Vec<f32> = vec![];
            for i in 0..pts.len() {
                let (x0, y0) = pts[i];
                let (x1, y1) = pts[(i + 1) % pts.len()];
                if (y0 <= y && y < y1) || (y1 <= y && y < y0) {
                    let t = (y - y0) as f32 / (y1 - y0) as f32;
                    xs.push(x0 as f32 + t * (x1 - x0) as f32);
                }
            }
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
            for pair in xs.chunks(2) {
                if pair.len() == 2 {
                    let a = pair[0].round() as i32;
                    let b = pair[1].round() as i32;
                    for x in a..=b {
                        self.set(x, y, c);
                    }
                }
            }
        }
    }

    /// 同色贪婪合并成矩形。元素数受 Hub v1 的 200 上限约束，合并越彻底越安全。
    fn emit(&self, e: &mut Vec<Value>, ox: i32, oy: i32, p: i32) {
        let mut used = vec![false; (W * H) as usize];
        for y in 0..H {
            for x in 0..W {
                let idx = (y * W + x) as usize;
                if used[idx] || self.cells[idx].is_none() {
                    continue;
                }
                let c = self.cells[idx].unwrap();
                let mut ww = 1;
                while x + ww < W {
                    let j = (y * W + x + ww) as usize;
                    if used[j] || self.cells[j] != Some(c) {
                        break;
                    }
                    ww += 1;
                }
                let mut hh = 1;
                'grow: while y + hh < H {
                    for k in 0..ww {
                        let j = ((y + hh) * W + x + k) as usize;
                        if used[j] || self.cells[j] != Some(c) {
                            break 'grow;
                        }
                    }
                    hh += 1;
                }
                for dy in 0..hh {
                    for dx in 0..ww {
                        used[((y + dy) * W + x + dx) as usize] = true;
                    }
                }
                e.push(json!({
                    "type":"rect",
                    "x": ox + x * p,
                    "y": oy + y * p,
                    "w": ww * p,
                    "h": hh * p,
                    "fill": c
                }));
            }
        }
    }
}

/// 身体与头部：所有状态共用，是「同一只猫」的基准。
fn body(g: &mut Grid, state: Status) {
    // 斗篷：轮廓比躯干宽、颜色比金属深一档，否则整只猫会糊成一团。
    g.poly(&[(7, 22), (23, 22), (22, 35), (8, 35)], CLOTH);
    g.poly(&[(11, 23), (19, 23), (18, 34), (12, 34)], DEEP);

    // 躯干：肩上一道亮边，代替描边勾勒轮廓。
    g.rrect(10, 22, 20, 33, 2, METAL);
    g.rect(10, 22, 20, 22, LIGHT);

    // 腰带与扣。
    g.rect(10, 28, 20, 29, DEEP);
    g.rect(14, 28, 16, 29, LIGHT);

    // 腿与脚。
    g.rect(12, 33, 14, 37, DEEP);
    g.rect(16, 33, 18, 37, DEEP);
    g.rect(11, 37, 14, 38, BLACK);
    g.rect(16, 37, 19, 38, BLACK);

    // 猫耳：耳尖必须高过帽檐，否则只剩两根「天线」。
    // 内耳用斗篷灰，比深灰柔和、比金属灰有对比。
    g.poly(&[(2, 13), (5, 1), (10, 13)], METAL);
    g.poly(&[(18, 13), (23, 1), (26, 13)], METAL);
    g.poly(&[(3, 12), (5, 4), (7, 12)], CLOTH);
    g.poly(&[(21, 12), (23, 4), (25, 12)], CLOTH);

    // 头。
    g.ellipse(14, 16, 10, 8, METAL);

    // 耳机挂在两侧，像耳罩。
    for cx in [3, 25] {
        g.circle(cx, 19, 2, DEEP);
        g.circle(cx, 19, 1, METAL);
    }

    // 面罩：脸中央整块黑，圆角给到 4 才看得出是护目镜而不是一块方块。
    g.rrect(7, 13, 21, 22, 4, BLACK);

    // 针织尖帽：只盖头顶中央，帽身收成尖、向一侧耷拉，末梢绒球。
    g.rrect(10, 5, 19, 7, 1, HAT);
    g.poly(&[(10, 6), (19, 5), (12, 0)], HAT);
    g.rect(10, 0, 12, 1, HAT);
    g.circle(9, 1, 1, LIGHT);
    // 帽檐上两道竖向织纹，比等距打点更像针织。
    g.rect(12, 5, 12, 7, DEEP);
    g.rect(17, 5, 17, 7, DEEP);

    let _ = state;
}

/// 表情：眼睛是主要区分手段。
fn face(g: &mut Grid, state: Status, phase: usize) {
    let lx = 10;
    let rx = 17;
    let top = 15;
    match state {
        Status::Working => {
            // 竖条发光眼，每 6 拍眨一次。
            let h = if phase == 5 { 2 } else { 5 };
            g.rect(lx, top, lx + 1, top + h - 1, WHITE);
            g.rect(rx, top, rx + 1, top + h - 1, WHITE);
        }
        Status::Waiting => {
            // 一只眼眯起，另一只睁大：像在等答复。
            g.rect(lx, top, lx + 1, top + 4, WHITE);
            g.rect(rx, top + 2, rx + 1, top + 4, WHITE);
            g.rect(rx, top, rx + 1, top, WHITE);
        }
        Status::Done => {
            // 闪光眼：3×3 的十字星，像素游戏里表达「高兴」的通用符号。
            // 试过 ^ ^ 与微笑弧，在 2 像素笔画下分别被读成皱眉和口罩。
            for cx in [lx + 1, rx + 1] {
                g.set(cx, top + 1, WHITE);
                g.rect(cx - 1, top + 2, cx + 1, top + 2, WHITE);
                g.set(cx, top + 3, WHITE);
            }
        }
        Status::Error => {
            // × 眼。
            for i in 0..4 {
                g.set(lx + i, top + i, WHITE);
                g.set(lx + 3 - i, top + i, WHITE);
                g.set(rx + i, top + i, WHITE);
                g.set(rx + 3 - i, top + i, WHITE);
            }
        }
        Status::Stopped => {
            // 闭眼：一条横线。
            g.rect(lx, top + 3, lx + 3, top + 3, WHITE);
            g.rect(rx, top + 3, rx + 3, top + 3, WHITE);
        }
        Status::Unknown => {
            // 半睁，瞳孔偏上，像在辨认。
            g.rect(lx, top + 2, lx + 1, top + 4, WHITE);
            g.rect(rx, top + 2, rx + 1, top + 4, WHITE);
        }
    }
}

/// 手臂与道具。剑一直握着，靠剑的高度与倾斜表达状态——比凭空长出
/// 两条举起的胳膊更像「同一只猫」，也不会挡住耳机。
fn arms(g: &mut Grid, state: Status, phase: usize) {
    // 完成时把剑举高（胜利姿势），等待时剑垂下，其余保持战斗姿势。
    let lift = match state {
        Status::Done => -4,
        Status::Waiting => 3,
        _ => 0,
    };
    let _ = phase;

    let (y0, y1) = (13 + lift, 25 + lift);

    // 握剑的胳膊先落色，剑压在它上面，才像「握着」而不是「插在旁边」。
    g.rect(4, y1 + 2, 7, y1 + 3, METAL);

    g.rect(1, y0, 2, y1, LIGHT);
    g.rect(1, y0, 1, y1, WHITE);
    g.set(2, y0 - 1, LIGHT);
    g.rect(0, y1 + 1, 4, y1 + 2, DEEP);
    g.rect(1, y1 + 3, 2, y1 + 5, METAL);
    g.rect(1, y1 + 3, 1, y1 + 4, DEEP);
    // 剑柄上的手。
    g.rect(3, y1 + 3, 4, y1 + 5, METAL);

    // 盾：右手侧的小圆盾。外圈深、内圈中、十字用亮灰，否则整块糊成齿轮。
    g.circle(26, 28, 3, DEEP);
    g.circle(26, 28, 2, CLOTH);
    g.rect(26, 26, 26, 30, LIGHT);
    g.rect(24, 28, 28, 28, LIGHT);

    if state == Status::Done {
        // 剑尖的星光：庆祝感靠这一下，不靠多余的手臂。
        let (sx, sy) = (2, y0 - 2);
        g.set(sx, sy - 1, DEEP);
        g.set(sx, sy + 1, DEEP);
        g.set(sx - 1, sy, DEEP);
        g.set(sx + 1, sy, DEEP);
    }
}

/// 头顶装饰：状态符。
fn badge(g: &mut Grid, state: Status, phase: usize) {
    match state {
        Status::Working => {
            // 三颗上浮的像素点，像正在输出的片段。
            let lift = [0, 1, 2][phase % 3];
            g.set(24, 4 - lift, DEEP);
            g.set(27, 2 - lift, METAL);
            g.set(23, 1, CLOTH);
        }
        Status::Waiting => {
            // 问号。
            let x = 25;
            let y = 2;
            g.rect(x, y, x + 3, y, DEEP);
            g.set(x + 4, y + 1, DEEP);
            g.set(x + 4, y + 2, DEEP);
            g.set(x + 3, y + 3, DEEP);
            g.set(x + 2, y + 4, DEEP);
            g.set(x + 2, y + 6, DEEP);
        }
        Status::Done => {
            // 爱心：放在帽尖右侧，紧贴头顶，别飘到画面角落去。
            let x = 22;
            let y = 1;
            g.rect(x, y + 1, x + 1, y + 1, DEEP);
            g.rect(x + 3, y + 1, x + 4, y + 1, DEEP);
            g.rect(x, y + 2, x + 4, y + 3, DEEP);
            g.rect(x + 1, y + 4, x + 3, y + 4, DEEP);
            g.set(x + 2, y + 5, DEEP);
        }
        Status::Error => {
            // 感叹号。
            let x = 26;
            g.rect(x, 1, x + 1, 6, DEEP);
            g.rect(x, 8, x + 1, 9, DEEP);
        }
        Status::Stopped => {
            // z z：一大一小错开，3 像素宽的 Z 在墨水屏上会糊成一个点。
            for (dx, dy, w) in [(21, 2, 4), (26, 6, 3)] {
                g.rect(dx, dy, dx + w - 1, dy, CLOTH);
                for i in 1..w - 1 {
                    g.set(dx + w - 1 - i, dy + i, CLOTH);
                }
                g.rect(dx, dy + w - 1, dx + w - 1, dy + w - 1, CLOTH);
            }
        }
        Status::Unknown => {
            // 省略号：三个点必须用中灰，浅灰在小尺寸上会直接消失。
            g.set(23, 4, CLOTH);
            g.set(25, 4, CLOTH);
            g.set(27, 4, CLOTH);
        }
    }
}

/// 把当前状态的宠物画进场景元素。
pub fn draw(e: &mut Vec<Value>, ox: i32, oy: i32, p: i32, state: Option<Status>, phase: usize) {
    let state = state.unwrap_or(Status::Unknown);
    let mut g = Grid::new();
    body(&mut g, state);
    face(&mut g, state, phase);
    arms(&mut g, state, phase);
    badge(&mut g, state, phase);
    g.emit(e, ox, oy, p);
}

/// 宠物在画布上的尺寸，供布局计算留白。
pub const SIZE_W: i32 = W;
pub const SIZE_H: i32 = H;

#[cfg(test)]
mod tests {
    use super::*;

    fn count(state: Option<Status>, phase: usize) -> usize {
        let mut e = vec![];
        draw(&mut e, 254, 98, 4, state, phase);
        e.len()
    }

    #[test]
    fn every_state_stays_within_element_budget() {
        // 整个场景上限 200，界面文字与进度条约占 25，宠物留 150 的额度。
        for s in [
            None,
            Some(Status::Working),
            Some(Status::Waiting),
            Some(Status::Done),
            Some(Status::Error),
            Some(Status::Stopped),
            Some(Status::Unknown),
        ] {
            for phase in 0..6 {
                let n = count(s, phase);
                assert!(n > 20, "宠物必须真的画出东西，实际 {n} 个矩形（{s:?}）");
                assert!(n <= 150, "单个宠物不得超过 150 个元素，实际 {n}（{s:?}）");
            }
        }
    }

    #[test]
    fn states_are_visually_distinct() {
        let render = |s: Status| {
            let mut e = vec![];
            draw(&mut e, 254, 98, 4, Some(s), 0);
            serde_json::to_string(&e).unwrap()
        };
        let base = render(Status::Working);
        for s in [
            Status::Waiting,
            Status::Done,
            Status::Error,
            Status::Stopped,
            Status::Unknown,
        ] {
            assert_ne!(base, render(s), "{s:?} 必须与 Working 有可见差异");
        }
    }

    #[test]
    fn pet_fits_inside_canvas() {
        let mut e = vec![];
        draw(&mut e, 254, 98, 4, Some(Status::Working), 0);
        for v in e {
            let (x, y, w, h) = (
                v["x"].as_i64().unwrap(),
                v["y"].as_i64().unwrap(),
                v["w"].as_i64().unwrap(),
                v["h"].as_i64().unwrap(),
            );
            assert!(x >= 0 && y >= 0, "不能有负坐标：{v}");
            assert!(x + w <= 400 && y + h <= 300, "不能超出 400×300：{v}");
        }
    }
}
