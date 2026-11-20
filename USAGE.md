# workbuddy-pet — 使用说明

[ZECTRIX Note 4](https://wiki.zectrix.com/) 400×300 墨水屏像素宠物，跟随本机 WorkBuddy。

**第一次安装先按 [README 的快速开始](README.md#快速开始) 走一遍**（下载 → 改名 → `init` → 验收四行），
装不上时看 [README 的前置条件与排错](README.md#前置条件与常见坑)。这份文档是完整命令与行为细节，供查阅。

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

只在想改代码或自己出包时才需要；**只想用的话直接下 Release 的 exe，跳过本节**。

```powershell
cd workbuddy-pet
$env:CMAKE_POLICY_VERSION_MINIMUM = '3.5'
cargo test  --locked --release      # 60 项单元 + 13 项集成；1 项需真机，ignored
cargo build --locked --release
```

依赖：Rust（MSVC）、CMake、VS C++ Build Tools（SDK 的原生 Opus 需要）；系统字体
`C:\Windows\Fonts\msyh.ttc` / `msyhbd.ttc`（渲染中文）。SDK 以路径依赖引用同级的
官方克隆（`../zectrix-local-push-sdk`），本项目不复制、不修改它——两个仓库必须并排，
只克隆本仓库会编译失败。

## 常用命令

```bat
workbuddy-pet init                               首次引导：检查配置 → 连设备 → 预览 → 确认推送 → 自启
workbuddy-pet sessions                           列出本机会话及推导状态与清单进度（诊断）
workbuddy-pet skin list|set <id>|current         查看/切换 Buddy 图鉴造型（守护下一帧生效）
                                                 设备上长按上/下键也能换：短按翻任务、长按换造型
workbuddy-pet summary on|off|show                任务完成后翻页看总结（默认开）；off 不读正文
                                                 show 重新扫一次会话并打印分页结果
workbuddy-pet voice doctor [--wav 路径]          语音自检：解释器/脚本/模型就绪耗时，可试转一段
workbuddy-pet voice say --wav 路径               只转写一段 WAV，看识别结果与耗时
workbuddy-pet voice paste --text "..." [--send]  只测投递：粘到输入框（默认不回车）
workbuddy-pet voice targets|windows|focus        诊断：候选投递会话 / 可见的 WorkBuddy 窗口 / 能否抢到前台
workbuddy-pet voice preview [--two] [--out 路径] 本地渲染语音确认页，不连设备（调版式/出文档图）
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

六套造型共用表情与状态符，只换帽子/服装/道具：`pixel` 像素喵（默认，针织帽 + 光剑圆盾）、
`abacus` 金算喵（兜帽 + 算盘账本）、`fortune` 暴富喵（黑金外套 + 金条公文包）、
`astro` 星际喵（白色宇航服 + 猫爪旗）、`lens` 镜头喵（毛线帽 + 条纹衫背带裤 + 旁轴相机）、
`latte` 拿铁喵（针织帽 + 围巾马甲 + 一杯手冲）。`skin list` 看全部，`skin set astro` 或
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

它做五件事：

1. 把当前 exe 复制到 `%LOCALAPPDATA%\zectrix-workbuddy-pet\bin\workbuddy-pet.exe`，
   自启项指向这份**副本**而不是构建目录（`cargo clean` 不会把它弄丢）
2. 在 `%APPDATA%\...\Startup\zectrix-workbuddy-pet.vbs` 放自启项，隐藏窗口启动 `supervise`
3. 注册计划任务 `zectrix-workbuddy-pet`（每分钟跑一次 `supervise`，已在跑则忽略），
   让常驻进程由任务计划服务创建——这样它不属于 WorkBuddy 给 hook 建的作业对象，
   不会随回合结束被一起清掉。注册失败会明确打印，见下文
4. 立刻拉起看门狗（不必等下次登录），看门狗再拉起守护，有任务就开始推送
5. 把目标 IP 存进 `state.json`；`--target` 可省略，第一次连接时自动发现并记住地址

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
- 总结页里：`上/下` 翻页，`确认` 退出（末页再按 `下` 也退出）
- 语音确认页里：`上/下` 换投递目标，`确认` 粘贴；**已粘贴时再按 `确认` = 发送**

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
   ↓ 再短按一次确认
回车发送 → 回到主界面
   45 秒没按第二次 = 放弃发送，文字仍在输入框里
```

- **两步确认**：第一次只粘贴，第二次才发送。这样识别错了还能在电脑上改——
  第二次按确认**只发回车、不重新粘贴**，所以两段之间改过的字不会被覆盖。
  页面上黑条会从「确认键 粘贴到输入框」变成「确认键 发送这条消息」。
- **发送的焦点校验**：回车是无差别按键，焦点跑到别的窗口就可能替你在别处确认一个
  对话框。所以发送前要求输入框仍持有键盘焦点（`CurrentHasKeyboardFocus`），
  拿不到就如实报「发送失败：输入框没拿到键盘焦点」而不发。失败不丢草稿，
  点一下 WorkBuddy 再按一次即可。
- **已粘贴后 `上/下` 不再换会话**：文字已经在那个会话的输入框里，换目标只会
  让人以为投到了别处。这时按上下会提示「已粘贴，按确认键发送」。

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
- **只粘贴不发送、第二次才发**：识别错字还有机会在电脑上改。第一次按确认时先用
  `workbuddy://chat/<id>` 把目标会话切到前台，再写剪贴板 + `Ctrl+V`；第二次只回车。
- **投递不依赖前台**：窗口在副屏、焦点在别的程序时，Windows 会拒绝后台进程抢焦点。
  现在的顺序是「UIA `SetFocus` → 模拟 Alt 解除前台锁定 → 附加输入队列 →
  `SwitchToThisWindow`」，仍失败才退到「直接写输入框的 Value」（仅当框为空，
  不覆盖草稿）。所以副屏也能投。
- **识别引擎**：优先 faster-whisper（CTranslate2），模型 `large-v3-turbo`，开 VAD（切掉首尾静音）。
  项目术语用 `initial_prompt` 做弱提示——**不用 faster-whisper 的 hotwords**：实测它会把词表
  硬塞进结果，没说的词也会冒出来；`initial_prompt` 只影响解码倾向，不会凭空造词。
  没装 faster-whisper 时自动回退 openai-whisper。GPU 不可用就退 CPU int8。
- 录音短于 0.4 秒按误触处理；纯噪声会被判成「没语音」而不是回显提示词。
  识别为空且电平很低时按「没听清」处理，提示靠近设备。
- 首次要载模型（turbo 首次还要下载约 1.5 GB），之后每段 0.3～1.5 秒出字。
  转写进程随守护生命周期存在，不单独常驻。
- 环境自检：`voice doctor [--wav 路径]` —— 打印解释器、脚本、模型、引擎与耗时，
  可顺带试转一段音频。`voice paste --text "..."` 只测投递链路，不录音；
  `voice focus` 只验证能否把窗口抢到前台（不写字）；
  `voice preview --pasted` 渲染「已粘贴待发送」那一版，用于调版式。

## 任务总结

任务完成后可以把 AI 最后那段结论带到设备上翻看。

```
任务完成 → 完成页显示总结的第一行（任务名下面），右下角提示「↓ 看总结」
   ↓ 短按下键
总结页：标题 + 正文 7 行 + 页码
   ↑/↓ 翻页（最多 8 页）
   确认键退出（末页再按 ↓ 也退出）
```

- **取哪一段**：当前轮最后一条 `status=completed` 的助手消息，把 `content` 里的
  `output_text` 块按顺序拼起来。
- **怎么清洗**：去掉标题井号、`**`/`__`/反引号、链接地址（保留链接文字），
  整段丢弃代码块，表格分隔线整行跳过、数据行的竖线换成「 · 」。最后按 1200 字截断——
  400×300 一页 7 行，五页也就 700 字，再长没人翻。
- **不落盘**：`Event.summary` 标了 `#[serde(skip)]`，`state.json` 里没有正文，下一轮开始即清空。
  守护重启后一度不显示（没有缓存），下一次扫描会用新读到的正文刷新一次——同序号事件
  允许用总结刷新，否则升级后旧任务的完成页永远只有「任务已结束」。
- **关闭**：`workbuddy-pet summary off`。关掉后不读正文，完成页只显示「任务完成」，
  右下角也不提示。`summary show` 可本地预览当前任务的分页结果（会重新扫一次会话）。
- **定时任务不参与**：它们整条不上屏，自然也没有总结页。

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
  **不含任何助手回复正文**——总结带 `serde(skip)`，只活在内存里。
- `out\pet-*.png`：`preview` / `init` 渲染的预览图。
- `out\recordings\*.wav`：设备长按确认键录制的音频（16 kHz 单声道 PCM16），只在录音成功时写入。
- `watch.log`：守护与看门狗的输出，**只追加、不自动清理**。长期运行会到几 MB，属正常；
  排查完想清空直接删掉，下次启动会自动重建。
- `asr_worker.py`：首次转写时从 exe 释放出来的转写脚本，删掉也会重新释放。
- `bin\workbuddy-pet.exe`：自启用的副本，`autostart on` 时复制。
- `daemon.hb` / `watchdog.hb` / `daemon.lock`：心跳与单实例锁，用于判断谁在运行；可随时删除。

`state.json` 含设备地址，不要分享。`workbuddy-pet forget` 只删这一个文件（设置、任务记录、
提醒指纹），不动 exe、日志与录音；要彻底清干净见下一节的卸载步骤。

## 启动 / 停止 / 卸载 / 更新

### 计划任务（v0.4.4 起）

`autostart on` 除了写启动文件夹，还会注册一个计划任务 `zectrix-workbuddy-pet`：

| 项 | 值 |
|---|---|
| 动作 | `<安装目录>\bin\workbuddy-pet.exe supervise` |
| 触发 | 每分钟一次 |
| 多实例策略 | 默认 `IgnoreNew`——看门狗已经在跑时，这次触发直接被丢弃，不创建进程 |

**为什么要多这一个任务**：WorkBuddy 执行 UserPromptSubmit hook 时会把它派生的进程放进
一个作业对象（job object），回合结束就整组终止。hook 里跑的 `ensure` 如果直接拉起看门狗，
这份看门狗就活不过一个回合——日志里没有收尾、没有 panic、没有崩溃转储，只有「守护每隔
几分钟消失一次」。由任务计划服务创建进程则不会：父进程是 `svchost`，不属于那个 job。
恢复窗口因此是 1 分钟（触发间隔），而平时不产生任何额外进程。

`ensure` 在被 job 包住时会主动让位，不再拉一份短命的看门狗去占位。判断依据是两个：
`task.installed` 标记文件（注册成功才写）、以及 `IsProcessInJob` 的返回值。

停止与卸载时 `autostart off` 会 `schtasks /end` + `/delete` 并删掉标记；手工清也可以：

```powershell
schtasks /delete /tn zectrix-workbuddy-pet /f
```

`autostart on` 里的这条 `schtasks` 调用是唯一需要外部程序的一步。如果所在环境禁止调用
`schtasks`（企业策略、或某些终端沙箱），会打印注册失败，此时仍能用，只是回到旧行为：
守护会被 WorkBuddy 的回合清理，由下一次 hook 补。

- **启动**：`workbuddy-pet watch`（前台跟随，可配 `autostart on` 常驻）；临时用一次就
  `workbuddy-pet run`。
- **停止**：前台 `Ctrl+C`——向设备发送 `close()`、保存状态后退出，不自动重连。
  后台用 `workbuddy-pet autostart off`（先停看门狗再停守护，否则 10 秒内会被补回来）。
  在任务管理器里强行结束守护也可以，看门狗约 20 秒后会再拉起一份——要真正停就得用 `off`。
- **卸载**：`workbuddy-pet autostart off` → 删除 `%LOCALAPPDATA%\zectrix-workbuddy-pet`
  （`forget` 只是删 `state.json`，可省）；不触碰 WorkBuddy 与设备上的任何数据。
- **更新**（用 Release 的 exe）：
  1. `workbuddy-pet autostart off`
  2. 用新 exe 覆盖 `%LOCALAPPDATA%\zectrix-workbuddy-pet\bin\workbuddy-pet.exe`
  3. `workbuddy-pet autostart on --target <IP>`，再 `--version` 确认版本号变了

  直接 `autostart on` 也会尝试热替换，但旧进程仍占着副本时可能**静默沿用旧版**，所以先 `off` 更稳。
- **更新**（自己编译）：`cd ..\zectrix-local-push-sdk && git pull`，再回本项目
  `cargo build --locked --release`，然后用新 exe 跑 `autostart on`（会把副本换成新版本并重启看门狗）；
  状态缓存保留。

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

先看日志，比猜快得多（**`-Encoding UTF8` 不能省**，PowerShell 5.1 默认按 ANSI 读，中文会变乱码）：

```powershell
Get-Content "$env:LOCALAPPDATA\zectrix-workbuddy-pet\watch.log" -Tail 40 -Encoding UTF8
```

| 现象 | 处理 |
|---|---|
| 双击 exe 没反应，只弹出一屏用法 | 正常：不带参数只打印用法。在终端里跑 `.\workbuddy-pet.exe init` |
| PowerShell 报「无法加载模块 %LOCALAPPDATA%」 | 那是 cmd 的变量写法。PowerShell 用 `$env:LOCALAPPDATA`，且带空格/引号的路径前要加 `&` |
| 首次运行弹「Windows 已保护你的电脑」 | exe 没有代码签名：点「更多信息 → 仍要运行」 |
| `autostart status` 里「计划任务：未注册」 | 重跑 `autostart on` 看报错；退出码非 0 多为 `schtasks` 被策略拦住，见[计划任务](#计划任务v044-起)一节 |
| 连不上 / 超时 | 同局域网、IP 正确、设备开启「待机保持 Wi-Fi」、无其他程序占用 |
| 刚强杀过进程 | 等 15 秒让固件释放租约 |
| 中文显示为方块 | 确认 `msyh.ttc` / `msyhbd.ttc` 存在 |
| 设备停在旧画面 | 系统通知会临时覆盖约 12 秒；`push` 打印「已推送」即设备报告显示完成 |
| 等待不响铃 | 检查 `--no-sound`、`settings.sound_enabled`；重连后的同一条提醒不会重复响 |
| 名字不跟着改 | 侧边栏改名写数据库，最多 15 秒后同步；仍不变用 `sessions` 看读取结果 |
| `watch.log` 越变越大 | 只追加不自动清理，几 MB 无妨；删掉即可，下次启动自动重建 |
| 按 ↓ 进不了总结 | 只有「完成」态且这一轮有回复才可翻页；`summary off` 会关掉；用 `summary show` 确认能否取到 |
| 第二次按确认没发送 | 焦点被别的窗口抢了：屏幕显示「发送失败：…」，点一下 WorkBuddy 再按；文字仍在输入框 |
| 语音粘贴后屏幕自己回了主界面 | 45 秒没按第二次，草稿弃了但文字还在输入框，去电脑上补个回车 |
| 设备「连一下又断开」、日志反复「设备忙碌」 | 旧版可能同时跑了两份守护。`tasklist \| findstr workbuddy-pet` 看到两个 `watch` 就是它，升级到最新版后互斥体会挡住第二份 |
