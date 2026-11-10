# workbuddy-pet — 使用说明

[ZECTRIX Note 4](https://wiki.zectrix.com/) 400×300 墨水屏像素宠物，跟随本机 WorkBuddy。
首页与安装见 [README.md](README.md)，这里是完整命令与行为细节。

## 与上游的差异

复用 [zectrix-agent-pet](https://github.com/itopinion/zectrix-agent-pet) 0.3.4（MIT）的
状态模型、400×300 布局与提示音逻辑，做了几处本机化替换：

| 上游（Hub 版） | 本机版（workbuddy-pet） |
|---|---|
| ZECTRIX Hub 负责渲染/连接/音频 | 直接用 [zectrix-local-push-sdk](https://github.com/itopinion/zectrix-local-push-sdk) 局域网直连 |
| 读取 `~/.codex/sessions` 跟随 Codex | 读取 `~/.workbuddy` 跟随 **WorkBuddy** |
| macOS aarch64 | Windows x64（本机验证） |

## 状态来源

只读三处本机数据，全部只取元数据，**不读取、不保存、不上传对话正文、推理、工具参数或代码**：

| 来源 | 取什么 |
|---|---|
| `%USERPROFILE%\.workbuddy\projects\<项目>\<会话>.jsonl` | 轮次开始、`AskUserQuestion`/`ExitPlanMode`、助手消息状态、`todo_write` 清单条目 |
| `%USERPROFILE%\.workbuddy\logs\main.log` | `[TaskNotify] pending/completed`：提问与**权限确认卡**的权威信号 |
| `%USERPROFILE%\.workbuddy\workbuddy.db` | `sessions`（侧边栏会话名）、`session_usage`（上下文占用） |

状态映射（只认明确记录，不做推测）：

- 用户新轮次 → `正在工作`；普通文本提问、失败的工具调用都不会被猜成等待或失败
- 有未答复的 `AskUserQuestion` / `ExitPlanMode` → `等你处理`
- 主进程登记 pending（question / permission）→ `等你处理`，与侧边栏「待确认」徽章同源
- 会话尾部是已完成（completed）的助手消息、且静默 8 秒 → `任务完成`
- 助手消息被中断 → `已停止`
- 超过 15 分钟无更新 → `状态待确认`（绝不自动判成功）

进度百分比有两个来源，界面措辞严格区分：

- `任务清单 3/7 · 43%`——从本轮 `todo_write` 清单推算（已完成 ÷ 总数，取消项不占分母）；
  Agent 没写过清单就没有这个数字
- `上报 65%`——Agent 通过事件接口主动上报；未知进度不生成百分比
- 清单按轮次计，新轮次从零开始；进度变化本身就是一次画面变化

## 构建

```bat
cd workbuddy-pet
set CMAKE_POLICY_VERSION_MINIMUM=3.5
cargo test --locked --release      & rem 46 项测试
cargo build --locked --release
```

依赖：Rust（MSVC）、CMake、VS C++ Build Tools（SDK 的原生 Opus 需要）；系统字体
`C:\Windows\Fonts\msyh.ttc` / `msyhbd.ttc`（渲染中文）。SDK 以路径依赖引用同级的
官方克隆，本项目不复制、不修改它。

## 常用命令

```bat
workbuddy-pet init                               首次引导：检查配置 → 连设备 → 预览 → 确认推送 → 自启
workbuddy-pet sessions                           列出本机会话及推导状态与清单进度（诊断）
workbuddy-pet skin list|set <id>|current         查看/切换 Buddy 图鉴造型（守护下一帧生效）
                                                 设备上长按上/下键也能换：短按翻任务、长按换造型
workbuddy-pet voice doctor [--wav 路径]          语音自检：解释器/脚本/模型就绪耗时，可试转一段
workbuddy-pet voice say --wav 路径               只转写一段 WAV，看识别结果与耗时
workbuddy-pet voice paste --text "..." [--send]  只测投递：粘到输入框（默认不回车）
workbuddy-pet preview [--demo waiting] [--session <会话ID>] [--out 路径] [--skin <id>]
                                                 渲染 400×300 预览 PNG，不连设备；
                                                 --session 只看指定会话（不落盘）
workbuddy-pet scene [--demo 状态]                打印布局 JSON（调试版式）
workbuddy-pet check  --target 192.168.1.100      核对设备（型号/电量/信号），不推画面
workbuddy-pet sound  --status waiting|done       只播一次提示音（--volume 0-100）
workbuddy-pet push   [--demo 状态] [--format gray16|gray4|mono]
                                                 单次推送后立即断开
workbuddy-pet run    [--standby] [--volume 0-100] [--no-sound] [--no-animation]
                                                 常驻：跟随状态、按键翻页、提示音
workbuddy-pet watch  [--idle-grace 300]          守护：待命不占设备，有任务才占用（推荐日常）
workbuddy-pet supervise                          看门狗：常驻，守护不在就补一份（登录自启跑的就是它）
workbuddy-pet ensure  [--verbose]                一次性检查：守护不在就拉起（幂等）
workbuddy-pet autostart on|off|status            登录自启；on 会装副本、起看门狗、立即生效
workbuddy-pet where                              配置与输出路径
workbuddy-pet forget                             删除本机状态缓存
```

- 设备与电脑需同一局域网，设备上开启「设置 → 网络同步 → **待机保持 Wi-Fi**」。
- `--demo` 的演示画面右上角标注「演示」、底栏显示「演示数据」，不写入任务历史，
  也不代表真实任务进度。
- `--format` 默认 `gray16`（16 级灰阶）；`gray4`、`mono` 刷新更快但细节更少。

## 换造型

四套造型共用表情与状态符，只换帽子/服装/道具：`pixel` 像素喵（默认，针织帽 + 光剑圆盾）、
`abacus` 金算喵（兜帽 + 算盘账本）、`fortune` 暴富喵（黑金外套 + 金条公文包）、
`astro` 星际喵（白色宇航服 + 猫爪旗）。`skin list` 看全部，`skin set astro` 或
`skin set 星际喵` 切换，`skin current` 查当前。

选中值存在 `state.json` 的 `model.settings.pet_skin`。守护每帧从磁盘读一次，所以换肤
**不用重启守护、不用重新 `autostart on`**；如果是另一个进程刚写的，`save_store` 的合并
会以磁盘值为准，不会被守护内存里的旧快照盖回去。旧配置没有这个字段时默认 `pixel`。

想临时试看而不改设置：`preview --demo working --skin fortune --out .\fortune.png`，
`push` / `run` 同样接受 `--skin`。

## 设备占用策略

一台设备同一时刻只允许一个连接，所以守护按任务开关设备：

- **守护本身也只能有一份**。Windows 用命名互斥体（`Local\zectrix-workbuddy-pet-daemon`）
  抢占，第二份启动即退出并打印「已有守护在跑」。以前是「读锁文件 → 写锁文件」两步，
  没有原子性：`ensure` 拉起守护的同一秒，看门狗的第一拍也会拉一份，两份都读到空锁、
  都继续跑——两份守护共用一台设备只能互相抢连接，屏幕上就是「连上又断开」，
  日志里刷满「设备忙碌，稍后重试」。

| 阶段 | 守护在做什么 | 设备 |
|---|---|---|
| WorkBuddy 未启动 | 每 5 秒看一眼进程，不碰设备 | 完全空闲 |
| 待命（已启动、无任务） | 只读会话文件推断状态，**不连接** | 完全空闲 |
| 有任务在跑（工作） | 连接、推送、翻页、响铃 | 被占用 |
| 等你确认 / 等你回答 | 保持连接，画面停在等待页，响一次提示音 | 被占用（直到处理完） |
| 一轮答完（完成） | 再守 `idle_grace` 秒（默认 300） | 期内占用，期满交还 |

- **「等你确认」一直占用**：确认卡、提问都是在等人回电脑，设备就是这条通知的出口。
  旧版把它当「等下一句」，5 分钟就交还，结果确认还挂着、屏幕却断了，新记录又把它
  拉回来，反复连接/断开。
- 一轮答完（完成）后 5 分钟内没有新会话才交还，给连续追问留无缝衔接；
  墨水屏断电后保留最后一帧，所以交还设备不会让完成画面消失。
- 释放前还会确认「正在跟随的会话文件 30 秒内没再增长」，避免在对方还写着日志时
  把连接撤掉；释放后只有真的出现新记录才重连。
- `--idle-grace 120` 改宽限；`0` 表示任务一结束就交还。值存进 `state.json`。
- 还在跑（工作）的任务连续占用；这类任务静默超过 15 分钟按「状态待确认」同样交还。
  等待确认的任务不参与「状态待确认」降级——它不会自己往前走，但一直有效。
- 设备休眠导致的 `Timeout` 断开，只在任务仍活跃时按 15 秒步长退避重连，连续失败 5 次停下，
  等下一个任务再试，不与别的程序抢设备。
- 手动 `run` 默认一直占着（便于调试），加 `--standby` 才有交还行为。
- **定时任务（后台自动化）不占设备**：调度器发起的会话（`sessions.is_background_automation = 1`）
  整条不上屏、不算任务，已上屏的会在下一拍退场。它们按计划反复运行，若当普通任务跟随，
  设备会被长期占着交还不了；代价是这类会话的确认卡同样不提醒，定时任务要写成无人值守能自己跑完。

## 自动跟随

**首次部署的最后一步就是它**：不执行 `autostart on`，程序只在手动 `push` / `run` 时才有画面，
关掉终端就停了，设备不会跟着 WorkBuddy 更新。

```bat
workbuddy-pet autostart on --target 192.168.1.100
```

它做四件事：

1. 把当前 exe 复制到 `%LOCALAPPDATA%\zectrix-workbuddy-pet\bin\workbuddy-pet.exe`，
   自启项指向这份**副本**而不是构建目录（`cargo clean` 不会把它弄丢）
2. 在 `%APPDATA%\...\Startup\zectrix-workbuddy-pet.vbs` 放自启项，隐藏窗口启动 `supervise`
3. 立刻拉起看门狗（不必等下次登录），看门狗再拉起守护，有任务就开始推送
4. 把目标 IP 存进 `state.json`

**为什么登录项拉起的是看门狗、不是守护本身**：启动文件夹每次登录只执行一次。守护被
任务管理器结束、被新版本覆盖、或者登录项装好之后根本没重新登录过，都会出现「WorkBuddy
在跑、设备却没人推」——而且没有任何东西会把它补回来。所以常驻的是看门狗：

- 看门狗每 10 秒看一眼守护的心跳（`daemon.hb`，守护每 5 秒写一次）
- 心跳过期就重新拉起守护，实测**杀掉守护后约 20 秒自动恢复**
- 守护自己写 `daemon.lock`（PID）做单实例，两份同时被拉起时后来者退出

`autostart status` 会分别报看门狗与守护的 PID 和心跳年龄；`autostart off` 先停看门狗
再停守护（顺序反了会被立刻补回来），并移除登录项。

装好之后，想用副本再调命令，PowerShell 里这样调（cmd 也能用 `cd /d` 后同上写法）：

```powershell
cd "$env:LOCALAPPDATA\zectrix-workbuddy-pet\bin"
.\workbuddy-pet.exe autostart status
```

不切目录就得加调用运算符：`& "$env:LOCALAPPDATA\zectrix-workbuddy-pet\bin\workbuddy-pet.exe" autostart on`。
`"%LOCALAPPDATA%\...\workbuddy-pet.exe" autostart on` 这种 cmd 写法在 PowerShell 里会报
「表达式或语句中包含意外的标记」——PowerShell 把带引号的路径当字符串，不是命令。

推送节流：每 2 秒扫一次会话，只有状态、标题、进度或选中项真的变了才刷屏；
没有变化一帧都不发。`animation` 关闭时画面完全静止，仅状态变化或手动翻页才更新。

## 按键

- `上/下`：多任务时翻页（每页 4 个，末页循环回首页）；≤4 个时选择任务
- `确认`：恢复首页与自动跟随（**不会**批准 Agent 的任何操作）

列表顺序按会话标识固定，心跳和状态更新不会换位；同一会话只显示最新一轮。

## 语音输入

**只在设备被占用时可用**（有任务在跑）：录音必须由设备发起、且全程要持有连接，
空闲时设备已经交还、按键根本到不了程序。所以它不额外抢设备，任务结束就一起消失。

```
长按确认键说话（固件进录音，顶部显示「正在聆听 / 松开结束」）
   ↓ 松手
设备显示「正在识别…」→ 本机转写
   ↓
设备显示：识别文本 + 「→ 会话标题」
   短按确认 = 粘贴到该会话输入框（不回车）
   多个会话时短按上/下 = 换投递目标
   20 秒无操作 = 丢弃
```

- **可选会话怎么来的**：正在工作 / 等你确认的排最前，其余按最近活动补，最多 8 个
  （范围是 Watcher 的 12 小时扫描窗口，已答完的会话也能选）。设备上短按上/下切换，
  确认键粘贴。一个都没有才显示「→ 当前窗口」，直接贴到当前输入框。
  查看当前候选：`voice targets`。
- **投递到别的会话**：切会话走 `workbuddy://chat/<基础会话 id>`，等 1.2 秒再粘贴。
  注意 id 必须是基础 UUID——带轮次后缀（`…-u1794204456`）的深链不生效，文字会落回
  原来那个会话。

- **为什么必须在本机转**：设备只提供 16 kHz 单声道 PCM，SDK 没有语音识别，也没有
  「发语音消息」的接口；WorkBuddy 的输入框只接受文字，它自带的语音按钮是渲染进程
  内部功能，外部调不到。所以链路只能是「设备录音 → 本机 whisper → 文字 → 粘贴」。
- **只粘贴不发送**：识别错字还有机会在电脑上改。按确认时先用
  `workbuddy://chat/<id>` 把目标会话切到前台，再写剪贴板 + `Ctrl+V`。
- **投递不依赖前台**：窗口在副屏、焦点在别的程序时，Windows 会拒绝后台进程抢焦点。
  现在的顺序是「UIA `SetFocus` → 模拟 Alt 解除前台锁定 → 附加输入队列 →
  `SwitchToThisWindow`」，仍失败才退到「直接写输入框的 Value」（仅当框为空，
  不覆盖草稿）。所以副屏也能投。
- **识别引擎**：优先 faster-whisper（CTranslate2），模型 `large-v3-turbo`，
  开 VAD（切掉首尾静音）与 hotwords（项目术语：主分支、构建、投递、造型…）；
  没装 faster-whisper 时自动回退 openai-whisper。GPU 不可用就退 CPU int8。
- 录音短于 0.4 秒按误触处理；纯噪声会被判成「没语音」而不是回显提示词。
  识别为空且电平很低时按「没听清」处理，提示靠近设备。
- 首次要载模型（turbo 首次还要下载约 1.5 GB），之后每段 0.3～1.5 秒出字。
  转写进程随守护生命周期存在，不单独常驻。
- 环境自检：`voice doctor [--wav 路径]` —— 打印解释器、脚本、模型、引擎与耗时，
  可顺带试转一段音频。`voice paste --text "..."` 只测投递链路，不录音；
  `voice focus` 只验证能否把窗口抢到前台（不写字）。

## 等待处理时的画面

Agent 提问或权限确认卡在等时，设备显示**题干 + 一条框起来的指引**
「请回到电脑确认」，底部提示变为「设备仅作提示」，并播放提示音。
**不显示选项列表、也没有上下键选择**——设备无法替你点击电脑上的选项
（实测模拟键盘与 UI Automation 都不可行），列出选项只会让人误以为能在设备上选。

### 信号源

1. **`main.log` 的 `[TaskNotify] pending: session=<uuid> kind=permission|question`（唯一可靠）**：
   只在真弹确认卡时写，卡挂着时大约每分钟重发一次。会话在通知之后没有再写记录，
   就说明还停在卡上等人点；写了新记录就是已经放行（WorkBuddy 不写「已解除」，
   只能这样反推）。超过 3 分钟没有再重发，视为卡已散。
2. **会话记录**：写入类工具（`Write`/`Edit`/`mcp__*`）发起后 10 秒还没有结果，
   或 `AskUserQuestion`，直接算等待。
3. **界面读不到这张卡**：实测确认卡不在 Chromium 暴露的可访问性树里——同一时刻
   树里 271 个节点，没有任何卡上的文案（「允许写入」「拒绝」都不出现），所以
   「看界面有没有允许/拒绝按钮」这条路走不通，只会把真确认压掉。
4. **文件写入确认挂在 `Bash` 上**，不是 `Write` 工具：命令会写某个文件时弹
   「允许写入 / 本次会话内始终允许 / 拒绝」。只认写入工具会把这整类确认漏掉。

提示音与「等你处理」一致；确认完成后下一拍（≤2 秒）恢复。

### 为什么设备不能替你「点」电脑上的选项

- 选项卡片只接受鼠标点击。模拟键盘（`keybd_event` 发 Down/Enter）能送达 WorkBuddy 窗口，
  卡片毫无反应（2026-10-22 实测）；
- UI Automation 直接点按控件不可行：本机执行策略禁止 `Add-Type` / 反射加载 UIAutomation；
- 写会话文件「替用户回答」是伪造输入、会污染记录，不做。

## 提示音

- 「等你处理」= 双短音 200 ms，「任务完成」= 上行三连音 220 ms，默认音量 90%。
- **单条流上限约 360 ms（固件行为）**：本机 NOTE4（固件 3.7.2）单条播放流只能收 6 帧，
  实测超过 5 帧就返回「处理失败」，与音量、连接延时、发送节奏都无关；SDK 官方
  `examples/play.rs` 的 1 秒正弦同样失败。所以提示音压到 200 ms 左右。想更长只能拆流，
  但流之间设备要 2～6 秒才接受下一条，不划算。
- 只在状态真的变化时响；启动/重连的首扫静默，但若此刻正停在「等你处理」会补响一次。
- 同一条提醒只响一次（指纹存 `state.json` 的 `announced`），设备反复重连不重复吵人。

## 底部状态栏

- **左下角**：当前会话的上下文占用「上下文 40% · 剩 181K」，读 `session_usage.used/size`，
  每 15 秒刷新、整百分比变化才刷屏。设置页的「积分余额」是云端数据、本地无缓存，不显示。
- **右下角**：花絮短语，直接镜像 WorkBuddy 界面此刻显示的那条——准备阶段是状态行的
  「准备中 你在思考大局，我在打磨细节」，出字之后搬到状态气泡里（`已消耗 1.42` 与
  `不再显示` 之间），几秒换一条。这条文案随请求流式下发、只在渲染进程内存里，磁盘上
  没有任何副本，守护用 UI Automation 读主窗口取它：**每次确定要刷新页面时**采一次，
  和这一帧一起推上屏；没有独立的定时轮询——界面自己换了十几条而任务状态没变时，
  设备不跟着闪屏。读不到（窗口最小化、气泡被关掉）时回退到内置 20 条同风格短语轮换。
  注意那个位置偶尔也会显示产品提示（「按需开启连接器…」），设备照实显示——它跟随的
  就是界面文字，不做内容筛选。`workbuddy-pet quip` 可随时打印当前读到的这一条。

## 数据

`%LOCALAPPDATA%\zectrix-workbuddy-pet\`：

- `state.json`：设置、最近 100 条任务、设备地址、已提醒指纹；升级保留。
- `out\pet-*.png`：`preview` / `init` 渲染的预览图。
- `out\recordings\*.wav`：设备长按确认键录制的音频（16 kHz 单声道 PCM16）。
- `watch.log`：守护与看门狗输出（超过 1 MB 在下次启动时清空）。
- `bin\workbuddy-pet.exe`：自启用的副本，`autostart on` 时复制。
- `daemon.hb` / `watchdog.hb` / `daemon.lock`：心跳与单实例锁，用于判断谁在运行；可随时删除。

`state.json` 含设备地址，不要分享；卸载用 `workbuddy-pet forget` 删除。

## 启动 / 停止 / 卸载 / 更新

- **启动**：`workbuddy-pet watch`（前台跟随，可配 `autostart on` 常驻）；临时用一次就
  `workbuddy-pet run`。
- **停止**：前台 `Ctrl+C`——向设备发送 `close()`、保存状态后退出，不自动重连。
  后台用 `workbuddy-pet autostart off`（先停看门狗再停守护，否则 10 秒内会被补回来）。
  在任务管理器里强行结束守护也可以，看门狗约 20 秒后会再拉起一份——要真正停就得用 `off`。
- **卸载**：`workbuddy-pet autostart off` → `workbuddy-pet forget` → 删除本项目目录与
  `%LOCALAPPDATA%\zectrix-workbuddy-pet`；不触碰 WorkBuddy 与设备上的任何数据。
- **更新**：`cd ..\zectrix-local-push-sdk && git pull`，再回本项目
  `cargo build --locked --release`，最后用新 exe 跑一次 `autostart on`（它会把副本换成
  新版本并重启看门狗）；状态缓存保留。

## 已知边界

- WorkBuddy 会话文件与库表格式是逆向适配，不是官方稳定 API；格式变化会失效
  （表现为「状态待确认」，不会显示错误状态）。
- 墨水屏只在真实变化时刷新；连续变化会在 2 秒扫描间隔内合并成一次刷新。
- SDK 不含语音识别 / TTS / 大模型；录音由设备长按确认键触发，存成 WAV。
- `ResultUnknown` 视为「可能已刷新」，不重发同一帧、不重播音频。
- 清单进度依赖会话文件能读到 `todo_write`：超过 2 MB 的大会话首次导入只读尾部，
  历史轮次的清单不算；运行中的任务清单持续追加在尾部，不受影响。
- 同一设备只允许一个程序管理，切换前先退出旧的；强杀后需等约 15 秒。

## 排障

| 现象 | 处理 |
|---|---|
| 连不上 / 超时 | 同局域网、IP 正确、设备开启「待机保持 Wi-Fi」、无其他程序占用 |
| 刚强杀过进程 | 等 15 秒让固件释放租约 |
| 中文显示为方块 | 确认 `msyh.ttc` / `msyhbd.ttc` 存在 |
| 设备停在旧画面 | 系统通知会临时覆盖约 12 秒；`push` 打印「已推送」即设备报告显示完成 |
| 等待不响铃 | 检查 `--no-sound`、`settings.sound_enabled`；重连后的同一条提醒不会重复响 |
| 名字不跟着改 | 侧边栏改名写数据库，最多 15 秒后同步；仍不变用 `sessions` 看读取结果 |
| 设备「连一下又断开」、日志反复「设备忙碌」 | 旧版可能同时跑了两份守护。`tasklist \| findstr workbuddy-pet` 看到两个 `watch` 就是它，升级到 0.4.3 后互斥体会挡住第二份 |
