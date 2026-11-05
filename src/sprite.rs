//! 像素宠物：来自 WorkBuddy Buddy 图鉴的四只机器猫。
//!
//! 画在一张 30×40 的灰度点阵上，再按色块合并成矩形交给 `layout`。
//! 之所以不用圆角矩形直接画：墨水屏上像素风的识别度靠硬边和有限色阶，
//! 抗锯齿的圆角会让它看起来像 UI 图形而不是角色。
//!
//! 四套造型共用骨架（头、面罩、腿脚）与状态表达（表情、头顶符号），
//! 只换帽子/服装/道具/配色——这样每套造型的六种状态都自动成立，
//! 也不会因为换装把「同一只猫」画散。

use crate::model::Status;
use serde_json::{json, Value};

/// 可选造型。id 会写进 state.json，改名要同时做旧值迁移。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Skin {
    /// 8-bit 全栈冒险家：针织尖帽、绿光剑、圆盾、斗篷。
    #[default]
    Pixel,
    /// 连帽衫里的理财脑：兜帽、算盘、账本。
    Abacus,
    /// 黑金机能风：大耳机、亮色滚边、金条与公文包。
    Fortune,
    /// 白色宇航服：头盔、背包、猫爪旗。
    Astro,
}

impl Skin {
    pub const ALL: [Skin; 4] = [Skin::Pixel, Skin::Abacus, Skin::Fortune, Skin::Astro];

    pub fn id(self) -> &'static str {
        match self {
            Skin::Pixel => "pixel",
            Skin::Abacus => "abacus",
            Skin::Fortune => "fortune",
            Skin::Astro => "astro",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Skin::Pixel => "像素喵",
            Skin::Abacus => "金算喵",
            Skin::Fortune => "暴富喵",
            Skin::Astro => "星际喵",
        }
    }

    pub fn desc(self) -> &'static str {
        match self {
            Skin::Pixel => "针织尖帽 · 光剑与圆盾 · 8-bit 冒险家",
            Skin::Abacus => "连帽衫 · 算盘与账本 · 数字敏感型",
            Skin::Fortune => "黑金机能外套 · 大耳机 · 金条与公文包",
            Skin::Astro => "白色宇航服 · 头盔与背包 · 猫爪旗",
        }
    }

    /// 大小写不敏感，同时接受中文名，方便直接照着图鉴的名字敲。
    pub fn parse(value: &str) -> Option<Self> {
        let v = value.trim();
        Self::ALL
            .into_iter()
            .find(|s| s.id().eq_ignore_ascii_case(v) || s.name() == v)
    }
}

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

/// 腿脚：四套共用，颜色随服装走。
fn legs(g: &mut Grid, cloth: u8) {
    g.rect(12, 33, 14, 37, cloth);
    g.rect(16, 33, 18, 37, cloth);
    g.rect(11, 37, 14, 38, BLACK);
    g.rect(16, 37, 19, 38, BLACK);
}

/// 猫耳：先落色，之后被头/兜帽盖住，只留尖端。
fn ears(g: &mut Grid, outer: u8, inner: u8) {
    g.poly(&[(2, 13), (5, 1), (10, 13)], outer);
    g.poly(&[(18, 13), (23, 1), (26, 13)], outer);
    g.poly(&[(3, 12), (5, 4), (7, 12)], inner);
    g.poly(&[(21, 12), (23, 4), (25, 12)], inner);
}

/// 黑色面罩：四套共用的「机器猫」底子，表情都画在它上面。
fn visor(g: &mut Grid) {
    g.rrect(7, 13, 21, 22, 4, BLACK);
}

/// 身体与头部：同一套造型的所有状态共用，是「同一只猫」的基准。
fn body(g: &mut Grid, skin: Skin, state: Status) {
    match skin {
        Skin::Pixel => body_pixel(g, state),
        Skin::Abacus => body_abacus(g),
        Skin::Fortune => body_fortune(g),
        Skin::Astro => body_astro(g),
    }
}

/// 像素喵：斗篷 + 针织尖帽 + 耳机，是最初那版形象。
fn body_pixel(g: &mut Grid, state: Status) {
    // 斗篷：轮廓比躯干宽、颜色比金属深一档，否则整只猫会糊成一团。
    g.poly(&[(7, 22), (23, 22), (22, 35), (8, 35)], CLOTH);
    g.poly(&[(11, 23), (19, 23), (18, 34), (12, 34)], DEEP);

    // 躯干：肩上一道亮边，代替描边勾勒轮廓。
    g.rrect(10, 22, 20, 33, 2, METAL);
    g.rect(10, 22, 20, 22, LIGHT);

    // 腰带与扣。
    g.rect(10, 28, 20, 29, DEEP);
    g.rect(14, 28, 16, 29, LIGHT);

    legs(g, DEEP);
    ears(g, METAL, CLOTH);
    g.ellipse(14, 16, 10, 8, METAL);

    // 耳机挂在两侧，像耳罩。
    for cx in [3, 25] {
        g.circle(cx, 19, 2, DEEP);
        g.circle(cx, 19, 1, METAL);
    }

    visor(g);

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

/// 金算喵：深色兜帽把头包住，连帽衫 + 抽绳 + 口袋，剪影是圆的。
fn body_abacus(g: &mut Grid) {
    // 连帽衫：比斗篷方正，下摆敞平。
    g.rrect(8, 22, 22, 35, 3, CLOTH);
    g.rect(8, 22, 22, 22, LIGHT);
    // 兜帽外圈（深色大圆），之后用头盖住下半部，形成帽口。
    g.ellipse(14, 13, 11, 10, DEEP);
    ears(g, DEEP, CLOTH);
    g.ellipse(14, 16, 9, 7, METAL);
    // 帽檐压住额头，兜帽才不像光头。
    g.rect(7, 9, 21, 10, DEEP);
    g.rect(8, 10, 20, 10, CLOTH);
    visor(g);
    // 抽绳两根 + 胸袋。
    g.rect(11, 23, 11, 29, LIGHT);
    g.rect(18, 23, 18, 29, LIGHT);
    g.rect(12, 29, 18, 32, DEEP);
    g.rect(12, 29, 18, 29, LIGHT);
    legs(g, DEEP);
}

/// 暴富喵：近黑的机能外套，靠白色滚边和大耳机把轮廓提出来。
fn body_fortune(g: &mut Grid) {
    g.poly(&[(6, 22), (24, 22), (23, 35), (7, 35)], BLACK);
    g.rrect(10, 23, 20, 33, 2, DEEP);
    // 亮色横领 + 一条浅灰领带：黑外套上唯一的浅色块，识别度全靠它。
    // 早先用两片多边形画 V 领，按行合并后要十几块矩形；换成横条 + 领带
    // 只占 3 块，给六状态的表情与头顶符号留够元素预算。
    g.rect(10, 22, 20, 23, LIGHT);
    g.rect(14, 24, 16, 27, METAL);
    g.set(15, 28, METAL);
    g.rect(14, 30, 16, 30, WHITE);
    legs(g, BLACK);

    ears(g, DEEP, LIGHT);
    // 头用单层深灰 + 顶部一道金属亮边。双层椭圆（亮外圈套暗内圈）
    // 会把每一行的同色像素切成两三段，光头部就吃掉近 20 个元素。
    g.ellipse(14, 16, 10, 8, DEEP);
    g.rect(8, 10, 20, 10, METAL);
    // 大耳机：纯浅灰圆盘比像素喵大一圈，省掉内圈以免把圆盘切碎。
    for cx in [3, 25] {
        g.circle(cx, 18, 3, LIGHT);
    }
    visor(g);
    // 额前扫描条：横线眼之外，再做一道金色眉线。
    g.rect(8, 11, 20, 11, LIGHT);
}

/// 星际喵：白底上画白色宇航服，必须先铺深色描边，否则整个人会消失。
fn body_astro(g: &mut Grid) {
    // 背包从左侧露出一块。
    g.rrect(4, 23, 9, 33, 2, METAL);
    g.rect(4, 25, 9, 25, DEEP);
    // 宇航服主体。
    g.rrect(9, 22, 21, 34, 3, LIGHT);
    g.rect(15, 23, 15, 33, DEEP);
    g.rect(9, 27, 11, 28, DEEP);
    g.rect(18, 27, 21, 28, DEEP);
    // 胸前控制板 + 三颗指示灯。
    g.rect(11, 24, 14, 29, DEEP);
    for y in [25, 27] {
        g.set(12, y, WHITE);
        g.set(13, y, WHITE);
    }
    legs(g, LIGHT);

    // 头盔：深色外圈当描边，内圈浅灰当壳体。
    g.ellipse(14, 15, 11, 10, DEEP);
    g.ellipse(14, 15, 10, 9, LIGHT);
    // 猫耳穿出头盔，做成浅灰小三角。
    g.poly(&[(5, 8), (6, 3), (9, 7)], LIGHT);
    g.poly(&[(19, 7), (22, 3), (23, 8)], LIGHT);
    g.rect(10, 24, 20, 25, DEEP);
    visor(g);
    // 玻璃反光：一道短斜线，别盖住眼睛。
    g.set(8, 14, METAL);
    g.set(9, 14, METAL);
    g.set(9, 15, METAL);
    g.set(10, 15, METAL);
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

/// 完成时道具尖端的星光：庆祝感靠这一下，不靠多余的手臂。
fn sparkle(g: &mut Grid, sx: i32, sy: i32) {
    g.set(sx, sy - 1, DEEP);
    g.set(sx, sy + 1, DEEP);
    g.set(sx - 1, sy, DEEP);
    g.set(sx + 1, sy, DEEP);
}

/// 手臂与道具。道具一直握着，靠高度与倾斜表达状态——比凭空长出
/// 两条举起的胳膊更像「同一只猫」，也不会挡住耳机。
fn arms(g: &mut Grid, skin: Skin, state: Status, phase: usize) {
    // 完成时把道具举高（胜利姿势），等待时垂下，其余保持工作姿势。
    let lift = match state {
        Status::Done => -4,
        Status::Waiting => 3,
        _ => 0,
    };
    let _ = phase;
    match skin {
        Skin::Pixel => arms_pixel(g, state, lift),
        Skin::Abacus => arms_abacus(g, lift),
        Skin::Fortune => arms_fortune(g, lift),
        Skin::Astro => arms_astro(g, state, lift),
    }
}

/// 像素喵：光剑 + 圆盾。剑一直握着，抬起就是胜利。
fn arms_pixel(g: &mut Grid, state: Status, lift: i32) {
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
        sparkle(g, 2, y0 - 2);
    }
}

/// 金算喵：左手算盘、右手账本。算盘的横杆与算珠是它的识别点。
fn arms_abacus(g: &mut Grid, lift: i32) {
    g.rect(4, 26 + lift, 8, 27 + lift, CLOTH);
    // 算盘框：深色外框 + 浅色内板，中间一道横梁。
    g.rrect(0, 20 + lift, 7, 29 + lift, 1, DEEP);
    g.rect(1, 21 + lift, 6, 28 + lift, LIGHT);
    g.rect(1, 24 + lift, 6, 25 + lift, DEEP);
    // 三列算珠：上二下一，每颗 1 像素，2 像素以上会连成一片。
    for x in [2, 4, 6] {
        g.set(x, 22 + lift, DEEP);
        g.set(x, 27 + lift, DEEP);
    }
    g.rect(1, 30 + lift, 6, 30 + lift, DEEP);
    g.rect(7, 28 + lift, 8, 30 + lift, METAL);

    // 账本：书脊在右，三行横线，别画成一整块深色。
    g.rect(24, 25, 29, 33, LIGHT);
    g.rect(28, 25, 29, 33, DEEP);
    for y in [27, 29, 31] {
        g.rect(25, y, 27, y, DEEP);
    }
    g.rect(22, 28, 24, 30, METAL);
    if lift < 0 {
        sparkle(g, 3, 18 + lift);
    }
}

/// 暴富喵：左手托金条、右手拎公文包。金条用梯形，别当方块画。
fn arms_fortune(g: &mut Grid, lift: i32) {
    // 金条压低一点：贴着耳机画会和浅灰大圆盘连成一片，读不出是金条。
    g.rect(4, 30 + lift, 8, 31 + lift, DEEP);
    g.poly(
        &[
            (0, 27 + lift),
            (7, 27 + lift),
            (6, 31 + lift),
            (1, 31 + lift),
        ],
        LIGHT,
    );
    g.rect(1, 27 + lift, 6, 27 + lift, WHITE);
    g.rect(3, 29 + lift, 4, 29 + lift, HAT);
    g.rect(7, 30 + lift, 9, 32 + lift, METAL);

    // 公文包：提手 + 翻盖 + 搭扣。
    g.rect(25, 24, 28, 25, LIGHT);
    g.rect(24, 26, 29, 33, DEEP);
    g.rect(24, 26, 29, 27, LIGHT);
    g.rect(26, 28, 27, 29, LIGHT);
    g.rect(22, 29, 24, 31, CLOTH);
    if lift < 0 {
        sparkle(g, 3, 21 + lift);
    }
}

/// 星际喵：右手举猫爪旗，旗杆是一根贯穿全身的长直线。
fn arms_astro(g: &mut Grid, state: Status, lift: i32) {
    // 左手手套贴在身体左侧。
    g.rrect(3, 27, 7, 32, 1, LIGHT);
    g.rect(3, 27, 7, 27, DEEP);

    // 旗杆从 y=14 起：右上角 y<10 是状态符号区（问号/爱心/感叹号/Z），
    // 长杆一路顶到头顶会把感叹号读成杆上的装饰。
    let top = 14 + lift;
    g.rect(26, top, 26, 34, DEEP);
    g.rect(25, top, 25, 34, LIGHT);
    // 旗面：向右侧展开的三角旗，比矩形更像被风吹着。
    g.poly(
        &[
            (26, top + 1),
            (29, top + 3),
            (26, top + 6),
        ],
        CLOTH,
    );
    // 猫爪印：一颗掌 + 三颗趾，得用白色才压得住中灰旗面。
    g.set(27, top + 4, WHITE);
    g.set(27, top + 2, WHITE);
    g.set(28, top + 2, WHITE);
    g.set(28, top + 3, WHITE);
    g.rect(23, 30, 26, 32, METAL);
    if state == Status::Done {
        // 星光贴在旗面左侧，别飘到右上角和爱心/感叹号抢位置。
        sparkle(g, 23, top + 2);
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

/// 把当前造型与状态的宠物画进场景元素。
pub fn draw(
    e: &mut Vec<Value>,
    ox: i32,
    oy: i32,
    p: i32,
    skin: Skin,
    state: Option<Status>,
    phase: usize,
) {
    let state = state.unwrap_or(Status::Unknown);
    let mut g = Grid::new();
    body(&mut g, skin, state);
    face(&mut g, state, phase);
    arms(&mut g, skin, state, phase);
    badge(&mut g, state, phase);
    g.emit(e, ox, oy, p);
}

/// 宠物在画布上的尺寸，供布局计算留白。
pub const SIZE_W: i32 = W;
pub const SIZE_H: i32 = H;

#[cfg(test)]
mod tests {
    use super::*;

    fn count(skin: Skin, state: Option<Status>, phase: usize) -> usize {
        let mut e = vec![];
        draw(&mut e, 254, 98, 4, skin, state, phase);
        e.len()
    }

    fn all_states() -> [Option<Status>; 7] {
        [
            None,
            Some(Status::Working),
            Some(Status::Waiting),
            Some(Status::Done),
            Some(Status::Error),
            Some(Status::Stopped),
            Some(Status::Unknown),
        ]
    }

    #[test]
    fn every_skin_and_state_stays_within_element_budget() {
        // 整个场景上限 200，界面文字与进度条约占 25，宠物留 150 的额度。
        for skin in Skin::ALL {
            for s in all_states() {
                for phase in 0..6 {
                    let n = count(skin, s, phase);
                    assert!(n > 20, "宠物必须真的画出东西，实际 {n} 个矩形（{skin:?}/{s:?}）");
                    assert!(
                        n <= 150,
                        "单个宠物不得超过 150 个元素，实际 {n}（{skin:?}/{s:?}/拍 {phase}）"
                    );
                }
            }
        }
    }

    #[test]
    fn states_are_visually_distinct() {
        let render = |skin: Skin, s: Status| {
            let mut e = vec![];
            draw(&mut e, 254, 98, 4, skin, Some(s), 0);
            serde_json::to_string(&e).unwrap()
        };
        for skin in Skin::ALL {
            let base = render(skin, Status::Working);
            for s in [
                Status::Waiting,
                Status::Done,
                Status::Error,
                Status::Stopped,
                Status::Unknown,
            ] {
                assert_ne!(base, render(skin, s), "{skin:?} 的 {s:?} 必须与 Working 有可见差异");
            }
        }
    }

    #[test]
    fn skins_are_visually_distinct() {
        let render = |skin: Skin| {
            let mut e = vec![];
            draw(&mut e, 254, 98, 4, skin, Some(Status::Working), 0);
            serde_json::to_string(&e).unwrap()
        };
        for i in 0..Skin::ALL.len() {
            for j in i + 1..Skin::ALL.len() {
                assert_ne!(
                    render(Skin::ALL[i]),
                    render(Skin::ALL[j]),
                    "两套造型不能长得一样"
                );
            }
        }
    }

    #[test]
    fn every_skin_fits_inside_canvas() {
        for skin in Skin::ALL {
            let mut e = vec![];
            draw(&mut e, 254, 98, 4, skin, Some(Status::Working), 0);
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

    #[test]
    fn skin_ids_parse_back_and_unknown_is_rejected() {
        for skin in Skin::ALL {
            assert_eq!(Skin::parse(skin.id()), Some(skin));
            assert_eq!(Skin::parse(skin.name()), Some(skin));
            assert_eq!(Skin::parse(&skin.id().to_ascii_uppercase()), Some(skin));
        }
        assert_eq!(Skin::parse("nope"), None);
        assert_eq!(Skin::parse(""), None);
    }
}
