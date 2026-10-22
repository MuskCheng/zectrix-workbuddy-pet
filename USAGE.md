# workbuddy-pet — ZECTRIX Note 4 像素宠物（WorkBuddy 版）

复用 [zectrix-agent-pet](https://github.com/itopinion/zectrix-agent-pet) 0.3.4（MIT）的
状态模型、400×300 布局与提示音逻辑，做了两处本机化替换：

| 上游（Hub 版） | 本机版（workbuddy-pet） |
|---|---|
| ZECTRIX Hub 负责渲染/连接/音频 | 直接用 [zectrix-local-push-sdk](../zectrix-local-push-sdk) 局域网直连 |
| 读取 `~/.codex/sessions` 跟随 Codex | 读取 `~/.workbuddy/projects` 跟随 **WorkBuddy** |
| macOS aarch64 | Windows x64（本机验证） |

状态来源：只读 `%USERPROFILE%\.workbuddy\projects\<项目>\<会话>.jsonl` 的生命周期元数据
（轮次开始、`AskUserQuestion`/`ExitPlanMode` 结构化提问、助手消息状态、ai-title，
以及 `todo_write` 任务清单的条目数与状态）。
**不读取、不保存、不上传对话正文、推理、工具参数或代码。**

状态映射（只认明确记录，不做推测）：

- 用户新轮次 → `工作`；普通文本提问、失败的工具调用都不会被猜成等待或失败
- 有未答复的 `AskUserQuestion` / `ExitPlanMode` → `等你处理`
- 文件尾部是已完成（completed）的助手消息 → `完成`
- 助手消息被中断 → `已停止`
- 超过 15 分钟无更新 → `状态待确认`（绝不自动判成功）

进度百分比（两个来源，界面措辞严格区分）：

- `任务清单 3/7 · 43%` —— 从本轮 `todo_write` 清单推算的完成项占比
  （已完成 ÷ 总数，取消项不占分母）；Agent 没写过清单就没有这个数字
- `Agent 上报进度 60%` —— Agent 通过事件接口主动上报的进度（演示画面即此来源）
- 进度变化本身就是一次画面变化，会触发刷新；清单按轮次计，新轮次从零开始

## 构建

```bat
cd workbuddy-pet
set CMAKE_POLICY_VERSION_MINIMUM=3.5
cargo test --locked        & rem 19 项单元测试
cargo build --locked --release
```

依赖：Rust (msvc)、CMake、VS 生成工具（SDK 的原生 Opus 需要）；系统字体
`C:\Windows\Fonts\msyh.ttc` / `msyhbd.ttc`（渲染中文）。

## 常用命令

```bat
workbuddy-pet sessions                        列出本机会话及推导状态与清单进度（诊断）
workbuddy-pet preview [--demo waiting] [--session <会话ID>]
                                     渲染 400×300 预览 PNG，不连设备；
                                     --session 只看指定会话（不落盘）
workbuddy-pet check  --target 192.168.1.100  核对设备（型号/电量/色板），不推画面
workbuddy-pet sound  --status done --target … 只播一次提示音
workbuddy-pet push   [--demo 状态] [--target …] [--format gray16|gray4|mono]
                                     单次推送后立即断开
workbuddy-pet run    [--target …] [--volume 0-100] [--no-sound] [--no-animation]
                                     常驻：跟随状态、按键翻页、提示音
workbuddy-pet watch  [--target …]             守护：WorkBuddy 启动即连、退出即断（推荐日常用）
workbuddy-pet autostart on|off|status [--target …]
                                     开机自启：写启动文件夹，后台静默等待 WorkBuddy
workbuddy-pet where                           配置与输出路径
workbuddy-pet forget                          删除本机状态缓存
```

- 设备与电脑需同一局域网，设备上开启「设置 → 网络同步 → **待机保持 Wi-Fi**」。
- `--demo` 的演示画面右上角标注「演示」、底栏显示「演示数据」，不写入任务历史。
- 交互应用需要电脑程序持续运行；网页/终端关掉不影响 `run`。

## 自动跟随（WorkBuddy 一启动就推送）

一次设置，之后开机就有：

```bat
workbuddy-pet autostart on --target 192.168.1.100
```

它做三件事：

1. 在 `%APPDATA%\...\Startup\zectrix-workbuddy-pet.vbs` 放一个自启项（隐藏窗口，不弹黑框）
2. 生成 `%LOCALAPPDATA%\zectrix-workbuddy-pet\watch.cmd`，内容带设备 IP，日志写到同目录 `watch.log`
3. 把目标 IP 存进 `state.json`

启动逻辑（`workbuddy-pet watch`）：每 5 秒检查一次 `WorkBuddy.exe` 是否存在

- **WorkBuddy 出现** → 连接设备、推当前状态、开始跟随；每 2 秒扫一次会话，
  **只有状态、标题、进度或选中项真的变了才刷屏**——没有变化就一帧都不发
  （2026-10-21 改：不再有周期性重刷，也没有动画轮播；`animation` 设置仅让
  每次实际刷新的姿态略有不同）
- **WorkBuddy 退出** → 正常 `close()` 断开，回到待命，不再占用设备
- **会话异常结束而 WorkBuddy 还在**（例如设备被别的程序占用）→ 只记日志，等 WorkBuddy
  退出后才重试；不在同一会话里自动重连、不强抢设备

现在想立刻生效，不必重启系统：`workbuddy-pet watch`（或双击 `watch.cmd`）。

## 按键

- `上/下`：多任务时翻页（每页 4 个，末页循环回首页）；≤4 个时选择任务
- `确认`：恢复首页与自动跟随（**不会**批准 Agent 的任何操作）

## 等待处理时的画面

Agent 提问（AskUserQuestion 等）时，设备只显示**题干 + 一条框起来的指引**
「需要你处理 · 请回到电脑上确认」，底部提示变为「请回电脑处理 · 设备仅提示」，
并播放提示音。**不显示选项列表、也没有上下键选择**——设备无法替你点击
电脑上的选项（实测模拟键盘与 UI Automation 都不可行，详见下节），
列出选项只会让人误以为能在设备上选。

### 为什么设备不能替你"点"电脑上的选项

- 选项卡片只接受鼠标点击。模拟键盘（`keybd_event` 发 Down/Enter）能送达
  WorkBuddy 窗口，但卡片毫无反应（2026-10-22 实测定案）；
- 用 UI Automation 直接点按控件也不可行：本机执行策略禁止 `Add-Type` /
  反射加载 UIAutomation 程序集；
- 写会话文件"替用户回答"是伪造输入、会污染记录，不做。

## 启动 / 停止 / 卸载 / 更新

- **启动**：`workbuddy-pet watch --target 192.168.1.100`（后台跟随 WorkBuddy，可配合
  `autostart on` 开机自动跑）；临时用一次就 `workbuddy-pet run --target …`
- **停止**：Ctrl+C——程序会向设备发送 close、保存状态后退出；不自动重连，再次运行即可。
  后台守护模式用任务管理器结束 `workbuddy-pet.exe`（强杀后设备约 15 秒才释放租约，属固件行为）
- **卸载**：`workbuddy-pet autostart off`（删自启项与守护脚本）→ `workbuddy-pet forget` →
  删除 `workbuddy-pet` 目录与 `%LOCALAPPDATA%\zectrix-workbuddy-pet`；不触碰 WorkBuddy 与设备上的任何数据
- **更新**：进入 `zectrix-local-push-sdk` / `zectrix-agent-pet` 目录 `git pull`，
  本仓库 `cargo build --locked --release`；状态缓存保留。
  注意本机 `zectrix-local-push-sdk` 有 3 处本地改动（见下），`git pull` 冲突时可丢弃
- **数据**：`%LOCALAPPDATA%\zectrix-workbuddy-pet\state.json` 只存任务标题、状态与设置；
  录音存 `%LOCALAPPDATA%\zectrix-workbuddy-pet\out\recordings\*.wav`。请勿在 WorkBuddy
  会话标题里放敏感信息（标题会显示在设备上）

## 已知边界

- WorkBuddy 会话文件格式是逆向适配，不是官方稳定 API；格式变化会失效（表现为
  「状态待确认」，不会显示错误状态）
- 墨水屏只在真实变化时刷新；连续变化会在 2 秒的扫描间隔内合并成一次刷新
- SDK 不含语音识别/TTS；录音由设备长按确认键触发，`run` 会把录音存成 WAV
- **提示音上限 300 ms（已定位根因并修复）**：本台 NOTE4（固件 3.7.2）的单条播放流
  只能收 6 帧（360 ms），实测**超过 5 帧就回 RESULT 结束原因 7「处理失败」**——
  与音量（0/50/100 都一样）、连接后延时、发送节奏都无关。SDK 官方 `examples/play.rs`
  那段 1 秒正弦同样失败，所以不是 workbuddy-pet 的 bug。修法是把提示音压到 200 ms 左右：
  「等你处理」= 双短音 200 ms，「任务完成」= 上行三连音 220 ms，`sound` 现在稳定返回
  `Completed`。想更长只能拆成多条流，但流与流之间设备要 2～6 秒才能接受下一条
  （AUDIO_BEGIN 会重传），所以不做拼接
- 本机 `zectrix-local-push-sdk` 的 3 处本地改动（均为诊断/兼容，已 `cargo test` 通过）：
  1. `tests/lan.rs`：Windows 上 UDP `WSAECONNRESET` 导致的假失败
  2. `src/device.rs`：`ZECTRIX_DEBUG=1` 打印收发帧（定位“设备处理失败”靠它）
  3. `src/device.rs`：`ZECTRIX_PACE_MS` 可调音频发送间隔（默认仍是 60 ms）
- `ResultUnknown` 视为“可能已刷新”，不重发同一帧
- 清单进度依赖会话文件能读到 `todo_write`：Agent 用任务清单跟进度时自动出现；
  超过 2 MB 的大会话首次导入只读尾部（清单记录在中部的历史轮次不算），运行中的
  任务清单持续追加在尾部，不受影响

## 底部状态栏

- **左下角**：优先显示「积分余额 ◇x,xxx.xx」——基准来自 `workbuddy-pet balance 2023.15`
  （设置页的积分余额是云端数据，本地磁盘/数据库/日志均无缓存，守护读不到，
  只能手动登记，变动后重跑一次 `workbuddy-pet balance <新值>`；`workbuddy-pet balance` 不带参数清除）。
  没有基准时显示「积分已消耗 ◇x.xx」——本会话消耗，读 WorkBuddy 的
  `~/.workbuddy/workbuddy.db`（session_usage.credit_json 求和 ÷ 100），与界面
  「已消耗 ◇x.xx」同口径，随对话自动增长
- **右下角**：随任务变动轮换的拟人花絮短语（内置 16 条）。WorkBuddy 界面的
  等待文案是前端随机选的、磁盘上不存在，设备显示的是同风格轮换，不是逐字同步

## 写入确认提醒

- WorkBuddy 弹「是否允许写入」权限确认卡时，设备约 10 秒后切换为
  「等你处理」画面 + 提示音，等待行显示「有写入操作等你确认 · 请回电脑点允许」
- 依据：Write/Edit/MultiEdit/NotebookEdit 与 mcp__* 调用发起后超过 10 秒没有
  返回结果——这些工具正常执行都在几秒内完成，卡住基本就是确认卡在等
- 边界：权限请求不写会话记录、日志无事件，只能这样从侧面推断；写入大文件
  超过 10 秒会误报一次（点完允许或执行完成即自动恢复）

## 待确认提醒（权威信号源）

- WorkBuddy 主进程在 `~/.workbuddy/logs/main.log` 里登记任务待确认事件，
  侧边栏「待确认」徽章就是它驱动的：
  `[TaskNotify] pending: session=<uuid> kind=question|permission` /
  `[TaskNotify] completed: session=<uuid>`
- 守护增量 tail 该日志（首次只看最后 512 KiB），任何会话出现待确认——
  包括**别的会话**（如另一个项目窗口里的权限卡）——其任务都会升级为
  「等你处理」并成为设备展示的首选任务（current() 有 Waiting 优先逻辑），
  提示音 + 画面与提问等待一致；确认完成后立即恢复
- 权限确认卡完全不写会话 JSONL（截图实测），旧启发式（写入工具卡住 10 秒）
  保留作兜底；Bash 类命令的确认卡只有 main.log 信号能覆盖
- 附带修复：守护重启后首次导入的游标没记 seen_at/seen_seq，权限卡恰好
  出现在此刻且会话无新行时永远报不出等待——现在导入时会记下首扫位置

## 左下角：上下文占用

- 显示「上下文 40% · 剩 181K」——当前展示会话的上下文已用/窗口，
  读 `~/.workbuddy/workbuddy.db` 的 session_usage（used/size），本地精确、
  每 15 秒刷新、随对话实时增长；上下文快满就该新开会话或压缩
- 之前的「积分余额」是设置页的云端数据，本地无缓存（已实测搜遍），
  手填快照会失真，已下线；`workbuddy-pet balance` 只提示不再登记
