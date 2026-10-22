# WorkBuddy 宠物（ZECTRIX Note 4）

![宠物实机画面：正在工作中](docs/preview.png)

*实机画面原尺寸 400×300，未放大。上图为一次真实推送：任务进行中、宠物在工作、
底部是上下文占用与随机花絮。重新生成：`workbuddy-pet preview --out docs/preview.png`。*

把本机 WorkBuddy 的任务状态推到你桌上的墨水屏：一台 400×300 的电子纸显示一只像素宠物，
工作中它在动、需要你处理时它举牌、任务完成时它收工。程序在本机运行，直连局域网里的设备，
不经过任何云端。

- 状态来源：本机 `~/.workbuddy/projects/**/<会话>.jsonl` 与 `~/.workbuddy/logs/main.log`
- 设备通道：`zectrix-local-push-sdk`（zectrix-sdk 0.1.0）局域网直连
- 渲染：本机生成 400×300 PNG → SDK 按设备色板量化为 mono / gray4 / gray16（不重复抖色）

## 兼容性

| 项目 | 要求 |
| --- | --- |
| 项目要求 | 本地推送 SDK · 本机 Agent 状态 · NOTE4 |
| 设备 | ZECTRIX Note 4（`zectrix-s3-epaper-4.2`），画布 400×300 |
| 固件 | 3.7.2（`zectrix-sdk` 0.1.0 配套版本）。**不适用于 Note 4C**，也不承诺其他固件 |
| 系统 | Windows 10/11 x64（本项目实测 Windows 11 专业版 / i5-12500H） |
| 工具链 | Rust MSVC 工具链（实测 rustc 1.99.1）、CMake（实测 4.4.4）、VS C++ Build Tools |
| 网络 | 设备与电脑在同一局域网；设备上开启「设置 → 网络同步 → 待机保持 Wi-Fi」 |
| API Key | 本地直连**不需要**极趣云 API Key；本程序也不读取任何账号或密钥 |

不自动升级固件，不连接未指定的设备，不与其它程序抢同一台设备。

## 安装说明

### 参考与核查

- 接入参照：[zectrix-agent-pet](https://github.com/itopinion/zectrix-agent-pet) 及其
  [安装说明](https://github.com/itopinion/zectrix-agent-pet/blob/main/README.md)
- 本站核查日期：**2026-10-20**；参考提交：`2c9213dda1063f6f8a19a4470eed0b367d14b6ef`
- 本地推送 SDK：https://github.com/itopinion/zectrix-local-push-sdk
- SDK API 文档：https://github.com/itopinion/zectrix-local-push-sdk/blob/main/docs/API.md
- SDK 兼容性清单：https://github.com/itopinion/zectrix-local-push-sdk/blob/main/compatibility.json

> 上面的核查日期与提交是本项目写这份说明时核对过的基线。**执行前请重新核对当前版本**
> ——SDK、固件与接入参照都可能已经更新，接口以当前源码为准。

**接入方式**：使用公开 SDK 构建本地程序，再接入应用逻辑、渲染与设备事件；执行时按当前源码
核对接口和依赖。本项目**不复制、不修改 SDK**，只以路径依赖引用官方仓库。

### 第 0 步 · 先读 SDK 文档

先读当前的 SDK README、API 文档与兼容性清单。撰写本说明时核对到的事实：

| 项 | 值 |
| --- | --- |
| SDK 版本 | `zectrix-sdk` 0.1.0 |
| 配套固件 | 3.7.2（不自动升级） |
| 板型 | `zectrix-s3-epaper-4.2` |
| 画布 | 400×300，位深 1/2/4 bpp（mono / gray4 / gray16） |
| 传输 | 局域网 UDP 32342，单连接独占 |

核对实际设备型号后再动手；Note 4C 不在兼容范围内。固件由你自己按官方指引升级，
本程序既不检查也不改动固件版本。

### 第 1 步 · 识别环境、准备 SDK 与工具链

1. 识别系统与处理器：本项目在 **Windows 11 专业版 / x64 / 12th Gen Intel i5-12500H** 上构建与实测。
2. 检查本机是否已有 SDK 克隆：有就直接复用（本项目 `D:\repositories\zectrix-local-push-sdk`
   是指向官方克隆的目录联接，`mklink /J` 建的，不是副本）。
3. 缺少时克隆公开 SDK，并准备好 Rust、C/C++ 编译器与 CMake：

```bat
cd D:\repositories
git clone https://github.com/itopinion/zectrix-local-push-sdk.git
cd workbuddy-pet
```

SDK 通过路径依赖接入（见 `Cargo.toml`）：

```toml
zectrix-sdk = { path = "../zectrix-local-push-sdk" }
```

官方 SDK 放在哪都行，只要路径对得上；也可用目录联接指向已有克隆。原生 Opus 需要
C/C++ 编译器（Visual Studio C++ Build Tools）与 CMake。

### 第 2 步 · 构建与本机自测

```bat
REM SDK 侧：先确认示例能编过（本项目已核对）
cd ..\zectrix-local-push-sdk
cargo check --locked --examples

REM 本项目：锁定依赖跑测试，再出 release
cd ..\workbuddy-pet
set CMAKE_POLICY_VERSION_MINIMUM=3.5
cargo test --locked --release
cargo build --locked --release
```

`CMAKE_POLICY_VERSION_MINIMUM=3.5` 不能省：CMake 4.x 对 SDK 内置的旧版 Opus 构建脚本会报
「Compatibility with CMake < 3.5 has been removed」。这是 SDK README 给的处理办法。

产物：`target\release\workbuddy-pet.exe`。本项目已实现要求中的这几件事：

| 要求 | 本项目的做法 |
| --- | --- |
| 设备选择 | `check` / `init` 自动发现（5 秒）；多台时用 `--target` 指定；地址存 `state.json` |
| 单连接管理 | 一台设备同一时刻只允许一个连接：`Busy` 不重试、不抢占，先退旧程序 |
| PNG 渲染 | 本机生成 400×300 灰度 PNG，交给 SDK 量化，不做二次抖动 |
| 事件处理 | 常驻循环持续消费 `next_event()`：按键翻页、断线处理 |
| 配置保存 | `state.json` 存设备地址、灰阶、音量、宽限、提醒指纹；启动合并磁盘值，不覆盖 |
| 退出清理 | `Ctrl+C` / 正常退出都调 `close()`；`ResultUnknown` 不重发 |

SDK **不提供现成的插件加载器**，业务逻辑要自己接（下一步）。

### 第 3 步 · 复用插件业务逻辑并适配到本地程序

读接入参照的源码，复用业务逻辑（模型/布局/音效，MIT，见「许可」），再把原有的渲染、
设备事件与音频接口改成 SDK 调用——不是换掉依赖名就算接入：

| 原插件能力 | 在本地程序里的落点 |
| --- | --- |
| 任务状态模型 | `src/model.rs`：状态机、去抖、待确认、上下文占用 |
| 布局与绘制 | `src/layout.rs` + `src/raster.rs`：场景 → 400×300 灰度 PNG |
| 音效 | `src/sound.rs`：16 kHz 单声道 PCM16，单条 ≤300 ms |
| 设备与事件 | `src/device.rs`：`open` / `show_with_format` / `play` / `next_event` / `close` |
| 状态来源 | `src/workbuddy.rs`：只读本机 WorkBuddy 会话与主进程日志 |

适配中实际踩到的差异（都已处理）：

- **`ResultUnknown` 不是失败**：画面或声音的显示结果无法确认时，不重发、不重播。
- **单连接独占**：强杀进程后固件要约 15 秒才释放租约，这期间重连只会拿到 `Busy`。
- **断线是常态**：设备休眠会主动 `Timeout` 断开；只在任务仍活跃时退避重连，任务结束就交还。

### 第 4 步 · 功能实现与逐项测试

逐项测过并记录结果，可用范围以此为据：

| 功能 | 状态 | 验证方式与结果 |
| --- | --- | --- |
| 画面渲染 | ✅ | `preview` 出图 400×300，中文正常，双行标题不溢出 |
| 设备连接/选择 | ✅ | `check` 报型号、电量、RSSI；多设备时 `--target` 指定 |
| 状态跟随 | ✅ | 工作 / 等你处理 / 完成三态均实测上屏 |
| 按键翻页 | ✅ | `上/下` 翻页、`确认` 回首页；不改动电脑上的任务 |
| 提示音 | ✅ | 等待两短哔、完成上行三连音；同一条提醒只响一次 |
| 配置保存 | ✅ | `state.json` 落盘地址、灰阶、音量、宽限、提醒指纹 |
| 退出清理 | ✅ | `Ctrl+C` → `close()`；不自动重连（守护模式按策略重连） |
| 上下文占用 | ⚠️ 有限 | 读 `session_usage.used/size`；积分余额是云端数据，读不到，已改为显示占用 |
| 设备上点选项 | ❌ 不支持 | 选项卡只接受鼠标，程序只提示「请回电脑确认」 |
| 语音识别/TTS | ❌ 不涉及 | SDK 不含这些能力，本项目也不做 |

已知问题与边界见文末「已知边界」。

### 第 5 步 · 网络与设备占用

- NOTE4 与电脑需在可互通的局域网；设备上按 SDK 指引启用「待机保持 Wi-Fi」。
- 连接前先确认目标设备：`workbuddy-pet check --target <IP>`，并确认没有别的程序正在占用。
  **不抢占连接**：拿到 `Busy` 就停，等对方释放或换设备。
- 本地直连不需要极趣云 API Key。本项目不读取任何账号、密钥；数据源密钥按各插件单独配置。

### 第 6 步 · 推送与事件约定

- 提交**原始 400×300 PNG**，由 SDK 转 mono / gray4 / gray16，**不重复抖色**。
- 交互期间**持续消费 `next_event()`**（按键、录音、断线），退出时调用 `close()`。
- **不自动重连、不抢占设备**；`ResultUnknown` 不得当作失败重新刷图或播放。
- 画面只在签名变化时推送——墨水屏刷新昂贵，重复推同一张图只会闪屏。

### 第 7 步 · 音频与模型能力边界

- SDK 不含语音识别、TTS 或大模型。需要语音时另配后端。
- 播放格式：**16 kHz 单声道 PCM16**；录音由设备按键触发（本项目未使用录音功能）。
- **只有用户确认目标与预览之后，才执行首次设备推送**——`init` 就是按这个要求设计的：
  先出预览图，输入 `y` 才上设备。

## 本项目接入步骤

1. **用 SDK 构建本地程序**：获取公开的本地推送 SDK，按电脑系统准备 Rust 与编译工具，
   先通过示例构建和本地测试，再实现设备连接与画面输出。当前 SDK 配套 NOTE4 固件 3.7.2。
2. **接入宠物与任务状态**：复用任务状态与宠物逻辑，在本地程序实现 PNG 绘制、按键翻页和
   提示音。把原运行时调用改接 SDK，核对本机 WorkBuddy 路径与状态读取方式，保留隐私设置。
3. **检查本机 WorkBuddy 跟随**：新安装默认开启本机 WorkBuddy 跟随；升级保留已有设置。
   确认跟随开关后，用一条测试任务检查工作、等待和完成状态。按需设置动画和音量；
   需要处理任务时回到对应 WorkBuddy 窗口。

## 首次初始化

第一次使用跑一次引导，它会把配置逐项摆出来，再带你连上设备：

```bat
workbuddy-pet init
```

流程（每一步失败都不影响其它步骤，修正后重跑即可）：

1. **检查现状**：数据目录、状态缓存里有几个任务、WorkBuddy 会话目录与进程、
   已保存的设备地址、跟随/提示音/动画/灰阶设置
2. **连接设备**：不带 `--target` 时自动发现（最多 5 秒）；连上后打印型号、电量、
   信号，并把设备报告的地址存进 `state.json`（此后各命令都不必再带 `--target`）
3. **生成预览**：渲染一张 400×300 PNG 到 `%LOCALAPPDATA%\zectrix-workbuddy-pet\out\pet-init.png`，
   **先打开图确认版式**
4. **确认后再推送**：输入 `y` 才会上设备；`ResultUnknown` 不重试
5. **按需开机自启**：输入 `y` 才写启动项

常见失败与处理：`设备忙碌` → 已有守护在占用，先停掉再初始化（强杀后等约 15 秒）；
`没有发现可用设备` → 检查同一局域网、设备已开「待机保持 Wi-Fi」；
`发现多台设备` → 用 `--target <IP>` 指定其一。

> `init` 会在**你确认之后**才推送第一张画面；在此之前设备不会被动到。

## 首次验证

用一条**明确标注的测试任务**走完整链路，不把演示状态当作真实任务进度：

| 环节 | 怎么做 | 判定 |
| --- | --- | --- |
| 构建 | `cargo test --locked --release` → `cargo build --locked --release` | 测试全绿、产物存在 |
| 预览 | `workbuddy-pet preview`（可加 `--demo working\|waiting\|done\|error`） | 图 400×300、版式正确；`--demo` 只在内存，不写历史 |
| 设备回执 | `workbuddy-pet push --target <IP>` | 打印「已推送：<状态> · <任务名>」即设备报告显示完成 |
| 实屏观察 | 看设备本体 | 画面与该状态一致；发一条测试任务，确认工作→等待→完成依次变化 |
| 翻页与提示音 | 按 `上/下`、`确认`；等一次「等你处理」 | 翻页生效、不回写电脑；等待/完成各响一次 |
| 占用与交还 | 任务结束后等 5 分钟 | 日志出现「释放设备，回到待命」，此时别的程序能连上 |

本项目自己的验证记录（2026-10-22，Windows 11 / 官方 SDK 0.1.0 / Note 4 固件 3.7.2）：

- **构建**：46 项测试通过；`cargo check --locked` 与 `cargo build --locked --release` 均通过
- **预览**：`preview` 出图正常，README 首图即该命令产物
- **设备回执**：`已推送：正在工作 · 完整测试全流程`（电量 96%，RSSI -38 dBm）
- **实屏**：设备显示与预览一致（任务名、宠物姿态、上下文占用、花絮）

## 快速开始

```bat
REM 0) 先核对设备：型号、电量、信号，不推画面
workbuddy-pet check --target 192.168.1.100

REM 1) 只看画面，不连设备（可加 --demo working|waiting|done|error 看状态版式）
workbuddy-pet preview
REM 输出：%LOCALAPPDATA%\zectrix-workbuddy-pet\out\pet-working.png

REM 2) 推一张当前状态，推完立即断开
workbuddy-pet push --target 192.168.1.100 --format gray16

REM 3) 常驻跟随：状态变化才刷屏，按键翻页，等待/完成播提示音
workbuddy-pet run --target 192.168.1.100

REM 4) 日常用守护：WorkBuddy 启动即待命，有任务才占用设备，任务结束后交还
workbuddy-pet watch --target 192.168.1.100
```

## 设备占用策略（重要）

一台设备同一时刻只允许一个连接，所以守护**不常驻占用**，而是按任务开关：

| 阶段 | 守护在做什么 | 设备状态 |
| --- | --- | --- |
| WorkBuddy 未启动 | 每 5 秒看一眼进程，不碰设备 | 完全空闲，随便别的程序用 |
| 待命（WorkBuddy 已启动、还没有任务） | 只读会话文件推断状态，**不连接设备** | 完全空闲 |
| 有任务（正在工作 / 等你处理） | 连上设备推送状态、按键翻页、等待/完成响铃 | 被守护占用 |
| 任务结束后 | 再守 `idle_grace` 秒（默认 300 = 5 分钟） | 5 分钟内没有新任务就 `close()` 交还 |

- **占有规则**：任务完成后 5 分钟内没有新任务下达，自动解除占用。这 5 分钟是给连续追问
  留的无缝衔接窗口——期内又发一条，就不用断开再重连；**完成画面不会因为交还设备而消失**：
  墨水屏断电后照样保留最后一帧。
- **调宽限**：`workbuddy-pet watch --idle-grace 120`（秒）。`--idle-grace 0` = 任务一结束
  就交还；值会存进 `state.json`，下次不用再带。
- **等你处理也算活跃任务**：确认卡没答完就一直占着，直到任务结束。但超过 15 分钟
  没有新动静会被判为「状态待确认」，此时同样交还设备——一张被遗忘的确认卡不该把设备
  锁到天荒地老。
- **断线不会反复抢设备**：设备休眠会导致 `Timeout` 断开，只在任务仍活跃时按 15 秒步长
  退避重连，连续失败 5 次就停下等下一个任务。
- 手动 `run` 默认一直占着（便于调试），需要同样的交还行为可加 `--standby`。

## 开机自动跟随（可选）

```bat
workbuddy-pet autostart on --target 192.168.1.100
```

会做三件事：

1. 在启动文件夹放 `zectrix-workbuddy-pet.vbs`（隐藏窗口，不弹黑框）
2. 生成 `%LOCALAPPDATA%\zectrix-workbuddy-pet\watch.cmd`（含设备地址，日志写 `watch.log`）
3. 把目标地址存进 `state.json`

设置完成后，登录 Windows 就会自动在后台等待 WorkBuddy；想立刻生效不必重启，
直接运行 `workbuddy-pet watch`。

## 启动 / 停止 / 卸载 / 更新

- **启动**：登录后自动（装了自启的话）；手动启动用
  `%LOCALAPPDATA%\zectrix-workbuddy-pet\watch.cmd` 或 `workbuddy-pet watch`。
  交互应用需要电脑上的程序持续运行——**推一张图成功不等于应用装到了设备上**。
- **停止**：前台运行时 `Ctrl+C`——会向设备发送 `close()`、保存状态后退出，不自动重连。
  后台守护用任务管理器结束 `workbuddy-pet.exe`；强杀后设备约 15 秒才释放租约（固件行为），
  这期间其它程序连不上属正常。
- **卸载**：`workbuddy-pet autostart off` → `workbuddy-pet forget` → 删除本项目目录与
  `%LOCALAPPDATA%\zectrix-workbuddy-pet`。不触碰 WorkBuddy 与设备上的任何数据。
- **更新**：`cd ..\zectrix-local-push-sdk && git pull`，再回本项目
  `cargo build --locked --release`。状态缓存保留；`state.json` 不会被覆盖。
- **数据**：`%LOCALAPPDATA%\zectrix-workbuddy-pet\state.json` 只存任务标题、状态、设置与
  设备地址。从早期 `pet4` 目录升级时会自动复制一份旧 `state.json`（只复制、不删源）。

## 命令

| 命令 | 作用 |
| --- | --- |
| `init` | 首次初始化：查现状 → 连设备 → 出预览 → 确认后推送 → 询问自启 |
| `preview` | 渲染 400×300 PNG，不连设备；`--session <ID>` 只看该会话 |
| `scene` | 打印布局 JSON（调试版式用） |
| `sessions` | 列出可跟随的会话、推导状态与清单进度（诊断） |
| `check` | 核对设备型号/电量/信号，不推画面 |
| `sound` | 只播一次提示音：`--status waiting\|done` |
| `push` | 单次推送后断开 |
| `run` | 常驻跟随：状态变化才刷屏、按键翻页、提示音；`--standby` 任务结束即交还并退出 |
| `watch` | 守护：WorkBuddy 一启动就待命（不占设备），有任务才占用，任务结束交还（推荐日常） |
| `autostart` | 开机自启开关 |
| `where` / `forget` | 显示路径 / 删除本机状态缓存 |

## 按键

- `上/下`：多任务时翻页（每页 4 个，末页循环回首页）；任务 ≤4 个时选择任务
- `确认`：回到首页、恢复自动跟随（**不会**批准 Agent 的任何操作）

## 状态与画面

| 状态 | 画面 |
| --- | --- |
| 正在工作 | 宠物在动；无清单时显示「当前实施状态：正在修改文件 / 正在执行命令 …」 |
| 有清单 | 「任务清单 3/7 · 43%」+ 进度条 |
| 等你处理 | 题干（最多 2 行）+ 徽章「请回到电脑确认」+ 右下「设备仅作提示」，并响铃 |
| 任务完成 | 收工姿态 + 「完成！」，响铃一次 |

左下角显示「上下文 45% · 剩 164K」——当前会话的上下文占用，读
`~/.workbuddy/workbuddy.db` 的 `session_usage.used/size`，每 15 秒刷新，
整百分比变化才刷新屏幕。右下角是随任务变动轮换的花絮短语。

「等你处理」覆盖两类等待，信号源是 WorkBuddy 主进程写进
`~/.workbuddy/logs/main.log` 的 `[TaskNotify] pending: session=<id> kind=question|permission`：
提问题与**写入确认卡**都会提醒，且不论待确认发生在哪个会话。确认完成后立即恢复。

### 什么时候会响

- **状态真的变了**才响：工作 → 等你处理、工作 → 完成。同一状态被重复上报（进度、
  实施状态文字抖动）不会重复响。
- **开机/重连补一次**：程序刚启动、或设备超时后重连时，第一遍扫描是静默的（避免把历史
  回放当成新通知）；但如果此刻屏幕上正停在「等你处理」，会立刻补响一次——否则确认卡
  在没有提示音的情况下出现在屏幕上。刚完成 1 分钟内的任务同样补响。
- **同一条提醒只响一次**：提醒指纹（任务 + 上屏文案）写在 `state.json` 的 `announced`
  字段里，所以设备反复超时重连不会反复响；换一张确认卡、换一个提问、换一个任务会再响。
  `--no-sound` 或 `settings.sound_enabled=false` 可整体关闭。

## 隐私与数据

- 只读 WorkBuddy 的会话元数据：任务状态、清单条目数与完成数、主进程的待确认事件
- 任务名与 WorkBuddy 侧边栏的**会话标题**保持一致（同一个 `ai-title`，改名会跟着变）；
  没有会话标题的旧会话退回本轮提问首行/项目名
- 除标题一行外，**不读取、不上屏**对话正文、工具参数、推理过程、文件内容、账号与密钥
- **账号、密钥、设备地址与个人数据只在本机配置**，不写进共享任务、不打印进日志
- 服务与设备均在本机/局域网内，程序不向外发起任何请求
- 注意：任务名会显示在设备上，别把敏感信息写进会话标题

## 已知边界

- **积分余额读不到**：设置页的「积分余额」是云端数据，本地磁盘/数据库/日志均无缓存
  （已逐处核对），设备上不再显示，改为显示可精确读取的上下文占用。
- **设备不能替你点电脑上的选项**：选项卡片只接受鼠标；模拟键盘能送达窗口但卡片无反应，
  UI Automation 被执行策略禁止。所以等待画面只提示「请回电脑确认」。
- **写入确认靠主进程事件**：权限卡不写会话记录，完全依赖 `main.log` 的 TaskNotify；
  若日志被清理或轮转到 512 KiB 之外，会回退到「写入工具卡住 10 秒」的启发式（可能误报一次）。
- **单连接**：同一设备同一时刻只由一个程序管理。切换程序前先退出旧的，否则会拿到 `Busy`
  或连接被拒；强杀后需等约 15 秒租约超时。
- **断线由 `watch` 重连，`run` 不重连**：设备休眠/空闲会主动断开（`Timeout`），`watch`
  只在任务仍活跃时按 15 秒步长退避重连、连续失败 5 次才停（不会和别人抢设备）；
  单独跑 `run` 时则直接退出，由你自己决定何时重来。重连后同一条等待不会重复响
  （见上文「什么时候会响」）。
- **`ResultUnknown` 不重试**：SDK 无法确认结果时，程序不会重新刷图或重播，避免重复动作。
- **提示音是设备提示音**，16 kHz 单声道 PCM16；SDK 不含 TTS/语音识别/大模型。

## 排错

| 现象 | 处理 |
| --- | --- |
| 构建时报 CMake < 3.5 兼容错误 | 设 `CMAKE_POLICY_VERSION_MINIMUM=3.5` 后重试 |
| `cargo test --locked` 报锁文件不一致 | 只在你确实改过依赖时去掉 `--locked` 更新 `Cargo.lock` |
| `check` 超时 / 连不上 | 确认同一局域网、IP 正确、设备开启「待机保持 Wi-Fi」、没有别的程序占用 |
| 刚强杀过进程，连不上 | 等 15 秒让固件释放租约 |
| 画面中文是方块 | 确认 `C:\Windows\Fonts\msyh.ttc`、`msyhbd.ttc` 存在 |
| 设备显示旧画面 | 系统通知会临时覆盖约 12 秒；`push` 成功表示设备报告显示完成 |

## 许可

MIT。模型/布局/音效逻辑复用自 [zectrix-agent-pet](https://github.com/itopinion/zectrix-agent-pet)
0.3.4（MIT），设备通道使用 [zectrix-local-push-sdk](https://github.com/itopinion/zectrix-local-push-sdk) 0.1.0。
