# Rust重写MobaXterm方案

**User:** Anonymous  
**Created:** 8/27/2026 14:47:34  
**Updated:** 8/30/2026 21:53:33  
**Exported:** 8/30/2026 21:57:45  
**Link:** [https://chatgpt.com/c/6a8fdd82-2fb4-83e8-b80a-466479af4766](https://chatgpt.com/c/6a8fdd82-2fb4-83e8-b80a-466479af4766)  

## Prompt:
2026/8/27 14:47:33

bro我又有疯狂的想法了，mobaxterm非常好用，平时我们调车的时候都是用它，但是有一个致命问题，就是它不支持linux，我们跑仿真的时候，同时需要给小车发信号，这种情况就爆炸了，而且如果想使用它编辑文件的话，还不能直接打开VS code，必须用它的那个很恶心的文本编辑器
我了解了一下他的技术站，似乎是C加加，我能不能使用rust全面重写一下，界面什么全是原生的，试着做一下多平台适配，最重要的是，假如一个专门针对于ros或ros2的功能，可以专门订阅那个master

## Response:
2026/8/27 14:47:33

这个想法其实非常对路，而且真正有价值的点不是“Rust 重写 MobaXterm”，而是做成一个 **面向机器人开发的跨平台终端工作台**。我先帮你把最关键的技术可行性掰开：终端/SSH/SFTP 是一层，ROS/ROS2 集成又是另一层；尤其你说的“订阅 master”，ROS1 和 ROS2 的实现逻辑完全不一样。

我已经确认了几个关键拼图：**Rust 端并不是“生态不够”**。SSH 有成熟的异步实现，终端可以直接复用 WezTerm/Alacritty 的核心，ROS2 也已经有能查 graph、订阅 topic 的 Rust client。真正要控制的是范围——别一上来把 RDP/VNC/X11/FTP 全抄齐，不然你会从“做神器”瞬间变成“维护半个操作系统”。

> **正在搜索网页**
>
> [GitHub - ros2-rust/ros2_rust: Rust bindings for ROS 2 · GitHub](https://github.com/ros2-rust/ros2_rust)
> [rosgraph.masterapi.Master](https://docs.ros.org/diamondback/api/rosgraph/html/rosgraph.masterapi.Master-class.html)
> [Command Line Interface (CLI)](https://code.visualstudio.com/docs/configure/command-line)
>
> **规划替代方案**
>
> 我会把目标定义为“独立实现相近工作流”，而不是复制专有程序。由于原软件源码未公开，我不会假定其确切语言；设计上采用跨平台桌面界面、终端会话、文件传输、远程连接、日志与机器人控制模块。
>
> **规划了替代方案**
>
> 
>
> **正在搜索网页**
>
> [Remote Development Tips and Tricks](https://code.visualstudio.com/docs/remote/troubleshooting)
>
> **已搜索 3 个网站**
>
> 
>
> **正在搜索网页**
>
> [GitHub - lapce/floem: A native Rust UI library with fine-grained reactivity · GitHub](https://github.com/lapce/floem)
> [floem 0.2.0 - Docs.rs](https://docs.rs/crate/floem/latest)
>
> **强化远程架构**
>
> 我会让桌面端通过 SSH 启动机器人侧代理，由代理访问实际 ROS 环境，再回传话题、参数、日志和诊断信息。这样能避免跨平台依赖、主机名解析、DDS 组播、防火墙等问题，并支持本地与远程并行运行。
>
> **强化了远程架构**
>
> 
>
> **已搜索 2 个网站**
>
> 
>
> 思考了 1m 36s

bro，这个想法我非常支持，而且我甚至觉得你**不应该把目标描述成“Rust 重写 MobaXterm”**。

更准确的定位应该是：

> **一个跨平台、原生 Rust、面向 ROS/ROS2 机器人开发的远程开发工作台。**
>
> MobaXterm 只是它的第一层功能。

这样一来，你不是在和 MobaXterm 拼“我也有 SSH”，而是在做一个目前确实很缺的东西：**Terminal + SSH + SFTP + VS Code + ROS Graph + Topic/Param/Launch 调试**全部塞进一个工作流里。

而且技术上，2026 年 Rust 生态已经足够支撑这件事。

---

# 1. Rust 完全能干，而且很适合

先纠正一个小地方：MobaXterm 的完整源码并没有公开，所以不能简单断言“它就是 C++ 写的”。官方许可证反而能看到不少 VCL、MoTTY、X.org 等历史组件；它本身是专有软件。([MobaXterm](https://mobaxterm.mobatek.net/license.html?utm_source=chatgpt.com))

因此我建议：

**不要研究怎么“翻译 MobaXterm 源码”。**

直接 clean-room：

```text
MobaXterm 的工作流
        ↓
重新设计架构
        ↓
Rust 实现
```

这样许可证也干净很多。

而 Rust 端基础设施其实已经很豪华了。

### Terminal

甚至都不用自己实现 ANSI/VT100/VT220 那堆地狱。

可以直接考虑：

```text
wezterm-term
```

它已经处理了：

- terminal escape sequence
- scrollback
- selection
- mouse / keyboard encoding
- OSC 8 hyperlink
- Sixel
- iTerm2 image
- 各种 terminal attributes

而且 MIT。([GitHub](https://github.com/wezterm/wezterm/blob/main/term/README.md?utm_source=chatgpt.com))

PTY 直接：

```text
portable-pty
```

它就是 WezTerm 那套跨平台 PTY 层，Windows / Unix 都可以抽象掉。([Docs.rs](https://docs.rs/portable-pty/latest/portable_pty/?utm_source=chatgpt.com))

所以你的 terminal 核心可以变成：

```text
Shell / SSH
    ↓ bytes
PTY / SSH Channel
    ↓
wezterm-term
    ↓ cells
GPU Renderer
    ↓
UI
```

根本不用从 `\x1b[31m` 开始手搓。

---

# 2. SSH 也不用自己造密码学轮子

Rust 已经有：

```text
russh
```

它现在支持：

- SSH Client
- Interactive PTY
- SFTP
- local port forwarding
- remote port forwarding
- Unix socket forwarding
- public key authentication

而且是 Tokio async。([GitHub](https://github.com/Eugeny/russh?utm_source=chatgpt.com))

所以：

```text
roboterm-ssh
├── ssh connection
├── authentication
├── pty channel
├── sftp
├── port forwarding
└── keepalive
```

完全可行。

这意味着 MobaXterm 最核心的体验：

```text
左边 SFTP
右边 Terminal
```

Rust 能原生完成。

---

# 3. VS Code 这个痛点甚至特别好解决

这是我觉得你这个项目比 MobaXterm 爽很多的地方。

假设你现在 SSH：

```text
ucar@192.168.1.10
```

然后 SFTP 浏览到：

```text
/home/ucar/2026-xunfei-race/src/xxx.py
```

你右键：

```text
Open in VS Code
```

你的程序可以直接执行：

```bash
code --remote ssh-remote+ucar-car \
  /home/ucar/2026-xunfei-race/src/xxx.py
```

VS Code 官方就支持这种 Remote SSH URI/CLI。([Visual Studio Code](https://code.visualstudio.com/docs/remote/troubleshooting?utm_source=chatgpt.com))

甚至：

```text
双击 .py
    ↓
检测 VS Code
    ↓
检测 SSH Host alias
    ↓
code --remote ssh-remote+ucar ...
    ↓
VS Code Remote SSH 打开
```

所以完全没必要：

> 双击文件 → MobaTextEditor → 血压上升

还可以设置：

```text
Default Remote Editor

○ VS Code
○ Zed
○ Cursor
○ Lapce
○ Internal Editor
○ Custom Command...
```

这一下产品体验就已经很香了。

---

# 4. 最关键的地方：ROS1 的 Master 可以做，但你要理解它到底是什么

你说的：

> “专门订阅那个 master”

方向对，但这里有一个很重要的概念。

## ROS1

ROS1 有：

```text
roscore
   ↓
ROS Master
默认 11311
```

Master 提供 XML-RPC API，可以拿：

```text
Nodes
Topics
Publishers
Subscribers
Services
Parameters
```

ROS 官方的 Master API 本身就是 XML-RPC。([ROS 文档](https://docs.ros.org/diamondback/api/rosgraph/html/rosgraph.masterapi.Master-class.html?utm_source=chatgpt.com))

所以你完全可以做：

```text
ROS
├── Nodes
│   ├── /move_base
│   ├── /camera_node
│   └── /line_follow
│
├── Topics
│   ├── /cmd_vel
│   ├── /scan
│   ├── /camera/image_raw
│   └── /odom
│
├── Services
└── Parameters
```

但是：

## Master 不传 Topic 数据

这是个关键点。

ROS1 大致是：

```text
              ROS Master
             /          \
       注册/发现          注册/发现
          /                \
 Publisher  ───────────→ Subscriber
             TCPROS
```

Master 是**注册中心/发现中心**。

真正的：

```text
/cmd_vel
/image_raw
/scan
```

数据是节点之间直接传的。

所以：

```text
访问 Master
```

可以实现：

```text
topic list
node list
service list
publisher/subscriber relationship
parameter server
```

但如果你要：

```text
topic echo
topic hz
topic bandwidth
topic pub
```

那你的程序本身就得成为一个 ROS Node。

---

# 5. 好消息：ROS1 已经有纯 Rust 实现

甚至这部分都有人铺路了。

现在有 `roslibrust_ros1`，已经实现了纯 Rust：

```text
XML-RPC
TCPROS
Publisher
Subscriber
```

官方说明就是 native ROS1 communication implementation。([Docs.rs](https://docs.rs/roslibrust_ros1?utm_source=chatgpt.com))

所以你可以：

```rust
ROS1Backend
    ├── MasterDiscovery
    ├── TopicSubscriber
    ├── TopicPublisher
    ├── ParamClient
    ├── ServiceClient
    └── GraphMonitor
```

这一层甚至完全可以不依赖：

```text
roscore CLI
rostopic
rosnode
rosparam
```

只要 Master 在就行。

这就已经很变态了。

---

# 6. ROS2 更有意思：它根本没有 Master

这里你的架构必须分开。

ROS2 是：

```text
没有 roscore
没有 ROS Master
```

而是依靠：

```text
DDS Discovery
```

每个 Node 参与 discovery。

所以界面不要写：

```text
ROS2 Master
```

而应该叫：

```text
ROS Context

ROS1
Master: http://192.168.1.10:11311

ROS2
Domain ID: 0
RMW: CycloneDDS
Participants: 14
```

ROS2 Rust 目前也已经相当能用了。

官方 `ros2_rust / rclrs` 已经支持：

- publisher
- subscriber
- services
- actions
- parameters
- QoS
- graph queries
- node/topic discovery
- dynamic messages

不过项目自己也明确说 API 还在快速演进。([GitHub](https://github.com/ros2-rust/ros2_rust?utm_source=chatgpt.com))

甚至还有更激进的：

```text
ros2-client
```

它是：

> RustDDS + pure Rust ROS2 client

不依赖 rclcpp，也不依赖 C++ DDS。现在已经支持 Topic、QoS、Services、Actions、Graph discovery、Parameters 等。([GitHub](https://github.com/Atostek/ros2-client?utm_source=chatgpt.com))

于是你的 backend 可以天然抽象成：

```rust
trait RosBackend {
    fn nodes();
    fn topics();
    fn services();
    fn parameters();

    fn subscribe();
    fn publish();

    fn graph();
}
```

下面：

```text
RosBackend
├── ROS1Backend
│   └── roslibrust
│
└── ROS2Backend
    ├── rclrs
    └── ros2-client
```

UI 根本不关心下面是哪一代 ROS。

---

# 7. 但我更推荐你做一个非常关键的架构：Remote Agent

这是我认为整个项目最值得认真考虑的设计。

不要让桌面程序强行直接加入小车 ROS 网络。

因为你迟早会碰到：

```text
ROS_MASTER_URI
ROS_IP
ROS_HOSTNAME

DDS multicast
ROS_DOMAIN_ID
RMW_IMPLEMENTATION
Firewall
VPN
NAT
hostname resolve
```

然后你就会开始怀疑人生。

所以我建议：

```text
┌────────────── PC ───────────────┐
│                                 │
│          RoboTerm GUI           │
│                │                │
│              SSH                │
└────────────────┼────────────────┘
                 │
                 │ encrypted tunnel
                 ▼
┌────────── Robot / Server ───────┐
│                                 │
│       roboterm-agent            │
│                                 │
│      ┌────────┬─────────┐       │
│      │ ROS1   │ ROS2    │       │
│      └────────┴─────────┘       │
│          │         │            │
│       Master      DDS           │
└─────────────────────────────────┘
```

甚至 agent 不用安装。

第一次连接：

```text
SSH 登录
↓
检测 ~/.cache/roboterm-agent
↓
没有
↓
通过 SFTP 上传一个 3 MB 二进制
↓
启动
```

然后建立 SSH tunnel：

```text
127.0.0.1:xxxxx
        ↓ SSH
roboterm-agent
```

这会带来一个巨大好处：

### Windows 完全不需要装 ROS。

你的 Windows：

```text
RoboTerm.exe
```

就能查看：

```text
Ubuntu 小车 ROS1 Noetic
```

或者：

```text
Ubuntu 仿真机 ROS2 Humble
```

而真正的 ROS library 运行在对端。

这比：

```text
我要在 Windows 上 native 支持所有 ROS distro
```

靠谱太多。

---

# 8. 然后你的 GUI 可以真正针对机器人开发

这才是杀手级功能。

我脑子里已经能看到界面了：

```text
┌──────────────────────────────────────────────────────────┐
│ RoboTerm        Local ▾       Robot: ucar@192.168.1.10 ● │
├─────────────┬────────────────────────────────────────────┤
│ Sessions    │ Terminal 1 │ Terminal 2 │ ROS             │
│             ├────────────────────────────────────────────┤
│ Local       │                                            │
│ └ Ubuntu    │   roslaunch ...                            │
│             │                                            │
│ Robot       │   [INFO] navigation started                │
│ └ UCar      │                                            │
│             │                                            │
│ Files       ├────────────────────────────────────────────┤
│ ├ src       │ ROS Monitor                                │
│ ├ launch    │                                            │
│ └ config    │ /cmd_vel        30 Hz                      │
│             │ /scan           10 Hz                      │
│ ROS         │ /camera         30 Hz                      │
│ ├ Nodes     │                                            │
│ ├ Topics    │                                            │
│ ├ Params    │                                            │
│ └ Services  │                                            │
└─────────────┴────────────────────────────────────────────┘
```

这东西对于调机器人是真的实用。

---

# 9. Topic 右键菜单都可以做得非常变态

例如右键：

```text
/cmd_vel
```

出现：

```text
Echo
Info
Hz
Bandwidth
Publish...
Plot
Record
Copy type
Show publishers
Show subscribers
```

点：

```text
Publish...
```

右边自动根据：

```text
geometry_msgs/Twist
```

生成：

```yaml
linear:
  x: 0.0
  y: 0.0
  z: 0.0

angular:
  x: 0.0
  y: 0.0
  z: 0.0
```

甚至识别 `/cmd_vel` 后直接出现：

```text
↑
←  STOP  →
↓
```

或者 WASD 控车。

这就已经不是 MobaXterm 赛道了。

---

# 10. ROS Graph 也可以直接内置

像 `rqt_graph`：

```text
/camera
   │
   ▼
/image_raw
   │
   ▼
/detector
   │
   ▼
/detections
```

ROS2 的 `rclrs` 已经支持 graph queries。([GitHub](https://github.com/ros2-rust/ros2_rust?utm_source=chatgpt.com))

而 Foxglove 其实也证明了这种工作流非常有价值，它现在就有 Node/Topic/Service graph panel。([Foxglove Docs](https://docs.foxglove.dev/docs/visualization/panels/topic-graph?utm_source=chatgpt.com))

但 Foxglove 的核心是：

> Robotics visualization.

你的核心则是：

> Robotics development workstation.

两个东西不冲突。

---

# 11. 还有一个我特别建议你做的功能：环境诊断

这玩意对比赛现场可能比 Graph 还好用。

右上角：

```text
ROS Health ●
```

点进去：

```text
ROS1
────────────────────
ROS_MASTER_URI
http://192.168.1.10:11311       ✓

ROS_IP
192.168.1.20                    ✓

Master                          ✓

Network latency
4 ms                            ✓

/cmd_vel
Publisher: 1
Subscriber: 1                   ✓

/camera/image_raw
Publisher: 1
Subscriber: 0                   ⚠

ROS2
────────────────────
ROS_DOMAIN_ID
0

RMW_IMPLEMENTATION
rmw_cyclonedds_cpp

DDS Participants
13

QoS conflicts
1 ⚠
```

甚至：

```text
⚠ /camera/image_raw

Publisher:
BEST_EFFORT

Subscriber:
RELIABLE

Potential QoS incompatibility
```

这种东西才是真正的机器人开发痛点。

---

# 12. 你甚至可以把“参数到底从哪儿来的”做成核心卖点

比如：

```text
/max_vel_x = 0.4
```

点一下：

```text
Current value        0.4

Sources
──────────────────
default.yaml         0.3
navigation.yaml      0.5
launch argument      0.4  ← effective
runtime update       none
```

以及：

```text
Effective value: 0.4
```

这个功能如果做好，对 ROS 项目绝对非常凶残--特别是 launch、yaml、runtime param 混起来之后，很多人调半天实际上调了个寂寞。

---

# 13. 本地仿真 + 远程真车，应该成为一级概念

你现在提到的痛点，本质其实不是：

> MobaXterm 没 Linux。

而是：

> **开发环境存在多个 Execution Context，但现在的工具根本不理解 Context。**

所以你的产品可以有：

```text
Contexts

🖥 Local
   Ubuntu 22.04
   ROS2 Humble

🚗 UCar
   192.168.1.10
   ROS1 Noetic

☁ Server
   10.0.0.13
```

每个 Terminal tab 左上角明确：

```text
[LOCAL]
```

或者：

```text
[UCAR]
```

然后：

```text
Ctrl+Shift+Enter
```

可以：

> Send command to...

```text
☑ Local
☑ UCar
☐ Server
```

类似 MobaXterm 的 MultiExec，但是懂机器人。

于是你可以：

```text
Local:
roslaunch gazebo simulation.launch

UCar:
rostopic pub /start std_msgs/Bool true
```

放同一个窗口。

这就是你描述的那个爆炸场景最直接的解决方案。

---

# 14. GUI 我反而不推荐 Electron/Tauri

既然你的目标就是：

> 我受够 WebView 了。

那这次干脆贯彻到底。

我目前最看好的有两个。

### 第一选择：Floem

```text
Rust
+
Floem
+
wgpu
```

它就是 Lapce 团队做的 Rust native UI，支持 Windows/macOS/Linux，GPU rendering，MIT；Lapce 自己也是 Rust + Floem + wgpu。([GitHub](https://github.com/lapce/floem?utm_source=chatgpt.com))

非常适合这种：

```text
IDE
Terminal
Split View
Tree
Dock
Editor-like UI
```

的东西。

缺点：

> 还在成长，API 会变。

官方也明确这么说。([GitHub](https://github.com/lapce/floem?utm_source=chatgpt.com))

---

### 第二选择：Slint

如果你更在乎：

```text
UI 漂亮
动画
声明式
设计器
成熟度
```

Slint 很舒服，而且 desktop 支持 Windows/macOS/Linux。([Slint](https://slint.rs/?utm_source=chatgpt.com))

不过它的 license 比 Floem 复杂一些，目前是 GPLv3 / Royalty-Free / Commercial 三种体系。([GitHub](https://github.com/slint-ui/slint/blob/master/LICENSE.md?utm_source=chatgpt.com))

如果你以后准备：

```text
MIT 开源
```

我个人反而更喜欢：

> Floem。

---

# 15. 注意“原生 UI”这里有两个概念

如果你说：

> 全原生

指：

```text
没有 Chromium
没有 WebView
没有 HTML
没有 CSS
没有 JS
```

那么：

```text
Floem
Slint
Iced
egui
```

都可以。

但如果你说的是：

```text
Windows 用 WinUI widget
Linux 用 GTK widget
macOS 用 Cocoa widget
```

那事情会复杂非常多。

而且 terminal 本身就必须大量自绘。

所以我建议你的定义是：

> **Native binary + native OS integration + GPU-rendered UI**

而不是：

> 每个按钮必须是操作系统自己的 Button。

这样性能、跨平台和 UI 一致性都会舒服很多。

---

# 16. 我会这样拆 workspace

千万别一个：

```text
src/main.rs
```

然后两个月后变成克苏鲁。

直接 Workspace：

```text
roboterm/
│
├── crates/
│   │
│   ├── roboterm-app
│   │
│   ├── roboterm-ui
│   │
│   ├── roboterm-terminal
│   │
│   ├── roboterm-pty
│   │
│   ├── roboterm-ssh
│   │
│   ├── roboterm-sftp
│   │
│   ├── roboterm-config
│   │
│   ├── roboterm-protocol
│   │
│   ├── roboterm-agent
│   │
│   ├── roboterm-ros
│   │
│   ├── roboterm-ros1
│   │
│   └── roboterm-ros2
│
└── Cargo.toml
```

依赖关系：

```text
                 roboterm-app
                      │
               ┌──────┴──────┐
               │             │
              UI           Core
               │             │
          Terminal      Connection
                            │
                 ┌──────────┴─────────┐
                 │                    │
                SSH                  ROS
                                      │
                               ┌──────┴─────┐
                               │            │
                             ROS1         ROS2
```

这样以后 ROS 根本可以变成 optional feature：

```toml
[features]
default = ["ssh", "sftp"]

ros1 = [...]
ros2 = [...]
serial = [...]
```

---

# 17. 第一版千万别做这些

MobaXterm 现在包含 SSH、SFTP、RDP、VNC、XDMCP、Telnet、Serial、X11 server 等一大堆东西。([MobaXterm](https://mobaxterm.mobatek.net/features.html?utm_source=chatgpt.com))

别管：

```text
RDP
VNC
FTP
Telnet
XDMCP
Embedded HTTP Server
Embedded FTP Server
X Server
```

特别是：

# 不要第一版写 X Server。

这东西很容易把你拖进历史协议考古现场。

Linux 本身已经有 X11/Wayland。

Windows 要 remote X11 的少数场景，第一版：

```text
Detect X server
↓
forward DISPLAY
```

就完事。

后面再说。

---

# 18. 我建议 MVP 就只有六件事

```text
① Local Terminal
② SSH Terminal
③ SFTP File Browser
④ Open Remote File in VS Code
⑤ Split Pane / Multiple Contexts
⑥ ROS Inspector
```

其中 ROS Inspector 第一版：

```text
Nodes
Topics
Services
Parameters

Topic:
    info
    echo
    hz
    pub
```

就已经足够形成项目特色。

第二阶段再上：

```text
Graph
Plot
Image View
Bag
Launch Manager
Parameter Editor
TF Viewer
Diagnostics
QoS Inspector
```

第三阶段：

```text
Serial
Port Forwarding UI
MultiExec
Macros
SSH JumpHost
Containers
Docker
Kubernetes
```

这时候你才开始真正吃 MobaXterm/Termius/Tabby 的功能范围。

---

# 19. 甚至可以做一个很变态的 Launch Manager

比如：

```text
Launch
────────────────────────────

full_competition.launch

Nodes
✓ speech
✓ camera
✓ navigation
✗ line_follow        exited 1

[Restart]
[Logs]
[Open source]
[Open launch file]
```

点：

```text
Open source
```

直接：

```text
VS Code Remote SSH
```

点：

```text
Logs
```

直接在下面开一个 terminal pane。

点 node：

```text
/line_follow
```

右边：

```text
PID
Host
Topics
Services
Params
CPU
RAM
stdout
stderr
```

这就已经有点：

> **ROS Task Manager + MobaXterm + VS Code launcher**

的味道了。

---

# 20. 市面上确实有部分“零件”，但没有完全撞你的定位

例如：

**Foxglove** 已经非常强，能实时看 topic、graph、image、plot、bag，ROS2 推荐使用 Foxglove Bridge。([Foxglove Docs](https://docs.foxglove.dev/docs/getting-started/frameworks/ros2?utm_source=chatgpt.com))

**PlotJuggler** 很强于时间序列和 ROS topic 数据分析。([ROS 文档](https://docs.ros.org/en/rolling/p/plotjuggler_ros/?utm_source=chatgpt.com))

甚至现在已经有人做 Rust 的 `ros2_tui`，可以看 Topic、Hz、Delay、Param。([GitHub](https://github.com/TillBeemelmanns/ros2_tui?utm_source=chatgpt.com))

但它们都不是：

```text
SSH
+
Terminal
+
Remote filesystem
+
VS Code bridge
+
ROS operations
+
Simulation/Robot contexts
```

这个组合。

所以你真正可以抢的位置不是：

> “我也能看 ROS Topic。”

而是：

# “我把机器人开发的整个工作流装进一个窗口。”

这个定位我认为是成立的。

---

## 最终我会选这一套

| 模块 | 技术 |
|---|---|
| Language | Rust |
| Async | Tokio |
| GUI | **Floem + wgpu** |
| Terminal model | **wezterm-term** |
| Local PTY | **portable-pty** |
| SSH | **russh** |
| SFTP | russh-sftp |
| Config | serde + TOML |
| Secret storage | OS Keychain |
| ROS1 | **roslibrust** |
| ROS2 | **rclrs / ros2-client** |
| Remote integration | **roboterm-agent** |
| VS Code | `code --remote ssh-remote+...` |
| Protocol | MessagePack/CBOR + framed Tokio stream |
| Release | cargo-dist / GitHub Releases |

而且这个项目有一个非常好的工程特点：

> **你完全没必要等 ROS 做完才能用。**

Terminal 做出来之后你自己就能开始拿它调车；SSH 做完，MobaXterm 使用频率就开始下降；SFTP + VS Code 做完，它已经能成为日常工具；最后再一点一点往里面加 ROS。

所以我甚至不建议 repo README 写：

> A MobaXterm clone written in Rust.

我会写成类似：

> **A native cross-platform robotics development workstation built in Rust.**

下面一句：

> Terminal, SSH, remote files, ROS/ROS2 introspection and robot debugging - in one workspace.

这样这个项目一下就从“学生复刻软件”变成了一个**有自己产品方向的开源项目**。

---

**Sources:**

- [MobaXterm Xserver and tabbed SSH client - license](https://mobaxterm.mobatek.net/license.html?utm_source=chatgpt.com)
- [wezterm/term/README.md at main · wezterm/wezterm · GitHub](https://github.com/wezterm/wezterm/blob/main/term/README.md?utm_source=chatgpt.com)
- [portable_pty - Rust](https://docs.rs/portable-pty/latest/portable_pty/?utm_source=chatgpt.com)
- [Remote Development Tips and Tricks](https://code.visualstudio.com/docs/remote/troubleshooting?utm_source=chatgpt.com)
- [rosgraph.masterapi.Master](https://docs.ros.org/diamondback/api/rosgraph/html/rosgraph.masterapi.Master-class.html?utm_source=chatgpt.com)
- [Topic Graph panel | Foxglove Docs](https://docs.foxglove.dev/docs/visualization/panels/topic-graph?utm_source=chatgpt.com)
- [Slint | Declarative GUI for Rust](https://slint.rs/?utm_source=chatgpt.com)

## Prompt:
2026/8/30 21:51:47

帮我起个炫酷的名字。英文的。
还有，既然是我自己开发，我打算加入诸如vscode的终端下拉菜单式命令补全或者类似这个项目[microsoft/inshellisense: IDE style command line auto complete](https://github.com/microsoft/inshellisense)  的补全。

## Response:
2026/8/30 21:51:47

这个方向一加进去，名字最好就别再像“SSH 工具”了，而应该有点 **机器人开发台 / 控制台 / 神经中枢** 的感觉。命令补全也很适合做成你的差异化卖点：终端不只是“显示 shell”，而是懂当前环境、远端主机和 ROS 上下文。

> **正在搜索网页**
>
> [inshellisense/README.md at main · microsoft/inshellisense · GitHub](https://github.com/microsoft/inshellisense/blob/main/README.md?plain=1)
>
> **已搜索 1 个网站**
>
> 
>
> 思考了 38s

这就更有意思了。加入 **IDE 式终端补全**以后，我觉得这个项目的定位已经彻底从“Rust 版 MobaXterm”升级成：

> **Robotics-native developer terminal / workstation**

所以名字也应该避开 `ROSxxx`、`SSHxxx` 这种功能型命名，不然以后加 Docker、Serial、AI Agent、Git、远端文件系统时，名字反而把自己限制住了。

我第一推荐：

# **MechaShell**

我很喜欢这个。

**Mecha + Shell**

既有机器人/机械感，又直接对应终端，而且读起来很顺：

> MechaShell - A native robotics development workstation.

CLI 本体甚至可以叫：

```bash
mecha
```

比如：

```bash
mecha connect ucar
mecha ros topics
mecha open .
mecha doctor
```

GitHub repo：

```text
mechashell
```

crate：

```text
mechashell-core
mechashell-terminal
mechashell-ssh
mechashell-ros
mechashell-agent
```

而且我刚刚简单查了一圈，没有看到一个明显占据这个名字的主流终端/机器人开发工具，至少比 `Nodal`、`NodePilot` 这种已经有明显项目撞名的名字干净得多。([GitHub](https://github.com/rogasp/nodepilot?utm_source=chatgpt.com))

---

## 我另外给你几个我觉得真能用的

| 名字 | 感觉 | 我的评价 |
|---|---|---|
| **MechaShell** | 机器人 + Shell | ★★★★★ |
| **MechaTerm** | 机器人终端 | ★★★★☆ |
| **VectorForge** | 运动/向量 + 工程工作台 | ★★★★★ |
| **IronNode** | 硬核、节点控制 | ★★★★☆ |
| **Nerve** | 机器人“神经中枢” | ★★★★☆ |
| **Synapse** | ROS 节点/通信/神经网络 | ★★★★☆，但撞名概率高 |
| **Actuate** | actuator/执行器 | ★★★★☆ |
| **Kinetix** | kinetic，运动控制 | ★★★★☆ |
| **Relay** | SSH/ROS/远程连接 | ★★★★☆，但太常见 |
| **MechaForge** | 机器人开发工坊 | ★★★★★ |
| **ArcShell** | 科技感很强 | ★★★★☆ |
| **Servo** | 机器人味极浓 | ★★★☆☆，过于常见 |

如果你想做得像：

```text
Warp
Ghostty
Zed
Helix
Lapce
```

这种**一个词、很有品牌感**的路线，我会进一步考虑：

> **Actuate**

或者：

> **Kinetix**

如果你希望别人第一眼就知道“机器人终端”：

> **MechaShell**

如果你希望以后产品范围扩得很大：

> **MechaForge**

我个人现在排序：

**MechaShell > MechaForge > VectorForge > Actuate。**

---

# 然后说你这个自动补全：必须做

而且我认为你**不要简单实现成 Bash Tab Completion**。

你应该直接做到这种：

```text
ucar@robot:~$ roslaunch ucar_2026_▌
                         ┌──────────────────────────────┐
                         │ ucar_2026_competition       │
                         │ ucar_2026_line_follow       │
                         │ ucar_2026_nav               │
                         │ ucar_2026_strict_mission    │
                         └──────────────────────────────┘
```

继续：

```text
$ roslaunch ucar_2026_competition ▌
                                   ┌─────────────────────────┐
                                   │ full_competition.launch │
                                   │ simulation.launch       │
                                   │ debug.launch            │
                                   └─────────────────────────┘
```

然后甚至：

```text
$ rostopic echo /▌
                  ┌─────────────────────────┐
                  │ /cmd_vel                │ geometry_msgs/Twist
                  │ /scan                   │ sensor_msgs/LaserScan
                  │ /camera/image_raw       │ sensor_msgs/Image
                  │ /odom                   │ nav_msgs/Odometry
                  └─────────────────────────┘
```

这时候你的 autocomplete 已经不是普通 terminal 能比的了。

---

# 你提到的 Inshellisense，其实非常值得“偷师”

Microsoft 的 `inshellisense` 并不是自己维护几百套 CLI 数据，而是把自己做成一个 autocomplete runtime，然后直接使用 **Fig/withfig 的 autocomplete specs**。

目前它官方介绍里就是：

> terminal-native runtime for autocomplete

并且利用 `@withfig/autocomplete` 支持 **600+ CLI 工具**。([GitHub](https://github.com/microsoft/inshellisense/blob/main/README.md?plain=1&utm_source=chatgpt.com))

而 Fig 的思路非常漂亮：

```text
git
├── add
├── commit
├── checkout
├── switch
└── push
```

被描述成一个 declarative completion spec：

```text
Command
├── Subcommands
├── Options
├── Arguments
└── Generators
```

官方就是这样定义 completion spec 的。([GitHub](https://github.com/withfig/autocomplete?utm_source=chatgpt.com))

例如一个 spec 大概描述：

```typescript
{
    name: "git",
    subcommands: [
        {
            name: "checkout",
            args: {
                generators: ...
            }
        }
    ]
}
```

而不是硬编码：

```rust
if command == "git" {
    if arg1 == "checkout" {
        ...
    }
}
```

后者迟早变成史诗级 `match` 地狱。

---

# 我建议 MechaShell 自己设计 Completion Engine

架构可以非常漂亮：

```text
                   User Input
                       │
                       ▼
               Command Parser
                       │
                       ▼
             Completion Engine
                       │
          ┌────────────┼────────────┐
          ▼            ▼            ▼
      Static Spec   Dynamic      Context
                    Provider      Provider
          │            │            │
          ▼            ▼            ▼
       git --       filenames       ROS
       docker       branches        SSH
       cargo        processes       Git
       cmake        env vars        Docker
```

Rust：

```rust
pub trait CompletionProvider {
    fn complete(
        &self,
        context: &CompletionContext,
    ) -> Vec<Suggestion>;
}
```

然后：

```text
CompletionProvider
├── SpecProvider
├── PathProvider
├── ExecutableProvider
├── HistoryProvider
├── GitProvider
├── RosProvider
├── DockerProvider
└── SshProvider
```

这才是我推荐的设计。

---

# 最变态的地方是：你可以做 Remote-aware Completion

这是普通 VS Code terminal 都很难替你完整解决的东西。

假设现在 tab 是：

```text
┌ UCar ────────────────────────────
ucar@192.168.1.10
```

输入：

```bash
cd /home/ucar/2026-▌
```

补全结果**必须来自小车**：

```text
2026-xunfei-race/
2026-test/
```

而不是本机。

输入：

```bash
roslaunch ▌
```

则 agent 在机器人上查询：

```text
ROS_PACKAGE_PATH
```

返回远程 package。

输入：

```bash
git checkout ▌
```

则查的是**远程 repo**的 branches。

于是你的 Completion Context 可以定义成：

```rust
struct CompletionContext {
    target: ExecutionTarget,

    cwd: PathBuf,

    shell: Shell,

    command_line: String,

    environment: Environment,

    ros_context: Option<RosContext>,
}
```

其中：

```rust
enum ExecutionTarget {
    Local,
    Ssh(SessionId),
    Container(ContainerId),
}
```

这设计一旦立住，后面扩 Docker 简直顺理成章。

---

# ROS Completion 可以成为你的独门绝活

例如：

```bash
rosnode kill /▌
```

直接从 ROS Master 获取：

```text
/move_base
/line_follow
/camera_node
```

---

输入：

```bash
rostopic echo /camera/▌
```

直接 graph query：

```text
/camera/image_raw
/camera/camera_info
/camera/image/compressed
```

---

输入：

```bash
rosservice call /▌
```

返回：

```text
/move_base/clear_costmaps
/global_localization
/reset
```

---

甚至：

```bash
rosparam set /move_base/▌
```

弹：

```text
max_vel_x
min_vel_x
acc_lim_x
yaw_goal_tolerance
```

这时候你就把：

```text
rostopic list
rosnode list
rosservice list
rosparam list
```

从**需要用户记住然后手动执行的命令**，变成了 IDE language service。

这个思路非常重要。

---

# 我甚至建议把它叫 Terminal Intelligence

你的 Terminal crate 不应该只是：

```text
mechashell-terminal
```

还可以有：

```text
mechashell-completion
```

里面分：

```text
mechashell-completion/
├── parser/
├── engine/
├── specs/
├── providers/
│   ├── filesystem.rs
│   ├── executable.rs
│   ├── history.rs
│   ├── git.rs
│   ├── ssh.rs
│   ├── ros1.rs
│   └── ros2.rs
└── ranking/
```

Suggestion 本身：

```rust
struct Suggestion {
    label: String,

    insert_text: String,

    description: Option<String>,

    kind: SuggestionKind,

    score: f32,
}
```

`kind`：

```rust
enum SuggestionKind {
    Command,
    Subcommand,
    Option,
    File,
    Directory,
    GitBranch,

    RosNode,
    RosTopic,
    RosService,
    RosParameter,
}
```

然后 UI 就能显示不同 icon：

```text
▣  /cmd_vel               geometry_msgs/Twist
◉  /move_base             ROS Node
ƒ  /clear_costmaps        Service
⚙  max_vel_x              Parameter
```

这看起来就已经很 IDE 了。

---

# 而且 Fig 那 600+ specs 我建议尽量复用

这是你最没必要重复造轮子的地方。

`withfig/autocomplete` 本身就是一大套 declarative specs，Inshellisense 正是在利用它。([GitHub](https://github.com/microsoft/inshellisense/blob/main/README.md?plain=1&utm_source=chatgpt.com))

相关 autocomplete tooling 代码也是 MIT。([GitHub](https://github.com/withfig/autocomplete-tools?utm_source=chatgpt.com))

所以一个很现实的路线不是：

> 我用 Rust 手写 600 个 command definitions。

否则你写：

```text
git
docker
cargo
cmake
python
pip
npm
pnpm
kubectl
ssh
scp
rsync
```

写到第 30 个命令就会开始思考人生。

而是设计：

```text
Fig Spec
   │
   ▼
Spec Compiler
   │
   ▼
MechaShell Completion IR
   │
   ▼
Rust Completion Engine
```

甚至构建阶段：

```text
*.ts Fig Specs
       ↓
conversion script
       ↓
completion-specs.bin
       ↓
include_bytes!()
       ↓
MechaShell
```

这样最终用户仍然只拿到：

```text
MechaShell.exe
```

**不需要 Node.js。**

Inshellisense 自己当前仍然是 TypeScript/Node 路线，依赖 `@withfig/autocomplete`、xterm headless、`node-pty` 等。([GitHub](https://github.com/microsoft/inshellisense/blob/main/package.json?utm_source=chatgpt.com))

而你完全可以：

> **兼容它的数据生态，但 runtime 用 Rust 重写。**

这就很舒服。

---

# 最后甚至可以有三级补全

我特别推荐：

### Level 1 - Spec Completion

```bash
git che▌

checkout
cherry-pick
```

---

### Level 2 - Runtime Completion

```bash
git checkout ▌

822
try/815
main
dev
```

来自真实 Git repo。

---

### Level 3 - Robotics Context Completion

```bash
rostopic echo ▌

/cmd_vel          geometry_msgs/Twist
/scan             sensor_msgs/LaserScan
/odom             nav_msgs/Odometry
```

来自 ROS graph。

而且 ROS suggestion 甚至可以显示实时状态：

```text
/cmd_vel
geometry_msgs/Twist
30.0 Hz · 1 pub · 2 subs
```

这就开始离谱了。

---

所以现在如果让我给这个项目写一句定位，我会直接用：

> **MechaShell**
>
> *A native, intelligent terminal and development workstation for robotics.*

然后 GitHub README 顶部五个 feature：

```text
Native Rust
SSH & SFTP
IDE-like command completion
ROS 1 / ROS 2
Local & Remote Workspaces
```

而且我认为 **IDE-like autocomplete 应该直接进 MVP**，不是锦上添花。因为 SSH + SFTP 只是让你追上 MobaXterm；**context-aware completion + ROS integration 才是让别人看到 MechaShell 后说“卧槽这个我真想装”的东西。** ([GitHub](https://github.com/microsoft/inshellisense/blob/main/README.md?plain=1&utm_source=chatgpt.com))

如果最终名字采用 **MechaShell**，我甚至已经能想象 Logo：一个非常简洁的 `>_` terminal prompt，左边的 `>` 被设计成机械臂/机器人关节的抽象形状。

---

**Sources:**

- [GitHub - rogasp/nodepilot: Operator-focused control panel for self-hosted Linux servers and Docker environments · GitHub](https://github.com/rogasp/nodepilot?utm_source=chatgpt.com)



---
Powered by [ChatGPT Exporter](https://www.chatgptexporter.com)