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
workbuddy-pet preview [--demo waiting] [--session <会话ID>] [--out 路径]
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

## 设备占用策略

一台设备同一时刻只允许一个连接，所以守护按任务开关设备：

| 阶段 | 守护在做什么 | 设备 |
|---|---|---|
| WorkBuddy 未启动 | 每 5 秒看一眼进程，不碰设备 | 完全空闲 |
| 待命（已启动、无任务） | 只读会话文件推断状态，**不连接** | 完全空闲 |
| 有任务（工作 / 等你处理） | 连接、推送、翻页、响铃 | 被占用 |
| 任务结束 | 再守 `idle_grace` 秒（默认 300） | 期内没新任务就 `close()` 交还 |

- 任务完成后 5 分钟内没有新任务才交还，给连续追问留无缝衔接；墨水屏断电后保留最后一帧，
  所以交还设备不会让完成画面消失。
- `--idle-grace 120` 改宽限；`0` 表示任务一结束就交还。值存进 `state.json`。
- 「等你处理」也算活跃任务；但超过 15 分钟没有新动静会判为「状态待确认」并交还设备。
- 设备休眠导致的 `Timeout` 断开，只在任务仍活跃时按 15 秒步长退避重连，连续失败 5 次停下，
  等下一个任务再试，不与别的程序抢设备。
- 手动 `run` 默认一直占着（便于调试），加 `--standby` 才有交还行为。

## 自动跟随

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

推送节流：每 2 秒扫一次会话，只有状态、标题、进度或选中项真的变了才刷屏；
没有变化一帧都不发。`animation` 关闭时画面完全静止，仅状态变化或手动翻页才更新。

## 按键

- `上/下`：多任务时翻页（每页 4 个，末页循环回首页）；≤4 个时选择任务
- `确认`：恢复首页与自动跟随（**不会**批准 Agent 的任何操作）

列表顺序按会话标识固定，心跳和状态更新不会换位；同一会话只显示最新一轮。

## 等待处理时的画面

Agent 提问或权限确认卡在等时，设备显示**题干 + 一条框起来的指引**
「请回到电脑确认」，底部提示变为「设备仅作提示」，并播放提示音。
**不显示选项列表、也没有上下键选择**——设备无法替你点击电脑上的选项
（实测模拟键盘与 UI Automation 都不可行），列出选项只会让人误以为能在设备上选。

### 信号源

- **权威**：`main.log` 的 `[TaskNotify] pending: session=<uuid> kind=question|permission`。
  权限确认卡完全不写会话 JSONL，Bash 等命令的确认卡也只有这里有信号；跨会话同样有效。
- **兜底启发式**：`Write`/`Edit`/`MultiEdit`/`NotebookEdit` 或 `mcp__*` 调用发起后
  超过 10 秒没有结果，判定为权限卡在等。`main.log` 被清理或轮转到 512 KiB 之外时才会用到，
  写入大文件可能误报一次，点完允许或执行完成即恢复。
- 提示音与「等你处理」一致；确认完成后立即恢复。

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
