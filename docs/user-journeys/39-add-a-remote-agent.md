# 说出它的名字，它就问："放我进来吗？"

**Persona:** 客厅的电视、另一个房间的手机、公司那台笔记本上的浏览器——手边没有键盘，也没有对着桌上那台机器的摄像头。
**Goal:** 不用走到 agent 跟前抄一串码，就把这台设备加进来。
**Preconditions:** agent 有名字（`iloahz.hi-agent.xyz`）；新设备能连上这个地址；人能看到 agent 的 `reach` 页（或任何已经进得去的设备）。

这是 [18](18-send-files-to-agent.md)/[36](36-show-your-screen-from-a-button.md) 的**前一步**：那些 journey 都写着"已配对一个 core"，这一条讲的就是那个"已"是怎么来的。

## Steps & expected UX

1. **新设备上打开 app / 打开地址** → 第一屏只问一件事：**这是哪个 agent**。一个输入框，旁边就贴着 `.hi-agent.xyz`——打 `iloahz`，整行拼成 `iloahz.hi-agent.xyz`，不用谁解释"名字"是什么。扫码和填完整地址都在下面，是备选，不是主路。
2. **按"Ask to be let in"** → 屏上出现一个**六位数**，一句"去 iloahz 上放行它"。没别的可做，人就举着这块屏。
3. **agent 那边的 `reach` 页顶上多出一条**：同一个六位数、这台设备自报的名字、多久之前问的，两个按钮——**放进来 / 不行**。码不是密钥、不授权任何东西；它只回答"同时有两台在等，哪台是我手里这台"。
4. **按"放进来"** → 那条从等待列表消失，设备出现在下面的设备列表里；新设备那边自己就进去了，不用再点一次。
5. **按"不行"** → 新设备读到的是"被拒绝了"，不是"超时"。两句话不一样，就不该长一个样。
6. **没人理** → 十分钟后自己过期，新设备说"没人应，再问一次"。

## Expected outcome

加一台设备 = 在新设备上说个名字，在 agent 那边点一下。全程没有人抄码、没有人跑到另一个房间、没有摄像头对着屏幕。

## 浏览器那条，顺带一起修了

一个浏览器停在 `https://iloahz.hi-agent.xyz/`，**core 一重启它就被挡在外面**——app 能拿钥匙串里的凭证重新换票，浏览器手里只有那块 cookie。而 cookie 上写着 `Max-Age=30 天`：服务端忘掉了它自己承诺会记住的东西。

现在 session 落在凭证旁边的表里，**重启照样进得去**；用过一天以上就顺手续到三十天，token 不换（换了会打断同一个页面里正在飞的那几个请求，而它换来的保证这里没人依赖）。一个月开一次，它就一直好使；放满三十天，它才重新问。

401 那一页也不再只有"填码"：**"让我进去"排在前面**——会落到这一页的，多半正是一个 session 过期了的浏览器，而它所在的机器不是 agent 跑着的那台。

## Edge cases & failure modes

- **名字没人认领 / 那边没醒** → "No agent answers to that name yet."（relay 给 404，core 睡着给 503，对人是同一件事）。
- **同时有八台在等，第九台** → 直接回绝（`429`），说"太多设备在等了，过几分钟再试"。上限本身就是防洪：这个接口不需要凭证，能堆多少就是陌生人能往人家屏幕上堆多少；而一个人一次也只加一台。
- **两台设备同时在等** → 两条各自带码，照着手里那块屏点。
- **凭证只发一次** → 新设备领走之后那条就没了，事后泄露的 secret 换不到东西。
- **agent 不会被告知有人在等** —— 这个接口不需要凭证，任何能通到 Cognition 的路都等于让陌生人支使别人的 agent 说话。等待只出现在 `reach` 页上。
- **撤销** → 撤掉设备，它的 session 跟着一起没，不是等它自己过期。

## 每种设备都走这条路了

iOS 先做、看过之后，其余四个客户端（Android 手机 / Android TV / Windows / Linux GTK）在
2026-09-11 铺开，**都是同一条名字优先的路**。macOS 的 Settings 窗口不在其中——它根本没有加设备
这件事，那里的 "credential" 指的是模型厂商的 key。

**电视是这件事最该做的那一台。** `TvPairScreen` 自己的注释写着"这是这个客户端唯一真正的代价"：
没有摄像头，所以打字不是退路而是唯一的路，而要打的是一个地址加 43 个字符的一次性码——用遥控器，
在屏幕键盘上。换成名字之后是六次左右按键，剩下的由手里那台有键盘的设备回答。

Linux 那边是另一种"终于合适"：一台无头机器没有摄像头可扫，读码意味着走到另一块屏幕前；而机架上
那台没有另一块屏幕。

每个客户端的形状都一样（名字打头、完整地址折叠在下面），差别只在工具箱：手机是 bottom sheet，
电视是整屏 + 焦点，Windows 是 `Expander`，GTK 是 `AdwPreferencesGroup` + `GtkExpander`。

---

## 实测 2026-09-11 · `1c48777` + 本改动（Mac mini，全新 `--data-dir`）

core 跑在 `--port 12381 --off-box 127.0.0.1:12382`，gate 只在 off-box 那个监听器上生效。

- ✅ **整条链路在真 core 上跑通**：off-box 匿名 `POST /api/access/request` → `reach` 列表里出现同一个码 → `approve` → 新设备用 secret 领到凭证 → `POST /api/session` 拿到 `Set-Cookie` → `GET /api/handle` `200`。
- ✅ **重启后 cookie 照样进得去**（这次改动就是为了它）：`200` → 杀进程重起 → 同一块 cookie 仍是 `200`。再 `DELETE /api/surfaces/{id}`，同一块 cookie `401`。
- ✅ **滚动续期走的是 gate**：新 session 用一次不写库、不回 cookie；把行倒签到"用了两天"之后再用一次，`Set-Cookie` 回来了，库里 `expires_at` 从 10-09 挪到 10-11，**用原来那块 cookie 继续 `200`**（证明没换 token）；再用一次又安静下来。
- ✅ **第九台被回绝**：八台在等时 `POST` 得到 `429`；拒掉一台就又能问了。
- ✅ **凭证只发一次**：第二次 poll 读到 `expired`。
- ✅ **`reach` 的"Waiting to be let in"在真页面上渲染**（iOS 模拟器的 WKWebView 里 + 桌面宽度的 headless Chromium 里各看过一遍）；请求落地后 **4 秒内**自己出现在页上，码和 core 发的一致。
- ✅ **两个按钮是在渲染出来的页面里点的**（CDP 驱动真实 DOM，不是绕过 UI 直接打接口）：点"放进来"→ 那条离开等待列表、设备出现在设备列表；点"不行"→ 那条离开等待列表、设备列表不变。
- ✅ **iOS 三个状态都看过**（iPhone 17 模拟器）：name（`iloahz` + `.hi-agent.xyz` 拼成一行）、waiting（大号六位数，轮询的是真 core）、以及 approved 之后 sheet 自己收走、stage 打开了 core 的 face。expired 那条文案也顺带撞见了一次，读着是对的。

**没看过的：**

- 🔲 **真机上的手机↔桌面放行**。以上都是模拟器 + 本机 loopback core，不是两台真设备隔着 relay。
- 🔲 **浏览器 401 页上的"让我进去"按钮**没在浏览器里点过——页面源码里的两个 fetch 路径有单测守着，但没人看它跑。
- ⚠️ **`make ios` 出来的包写不了 Keychain**：走到最后一步报 `-34018`（缺 entitlement），因为 `make ios` 带 `CODE_SIGNING_ALLOWED=NO`。改成 ad-hoc 签名（`CODE_SIGN_IDENTITY="-"`）就一路走通。**和本改动无关**（`KeychainStore` 没动，老的配对路径同样会撞上），但凡是在模拟器上验到"存凭证"这一步的，都得记得签一下。

---

## 实测 2026-09-11 · `9b756de` + 本改动（其余四个客户端铺开）

core 跑在 Mac mini 上 `--port 12391 --off-box 127.0.0.1:12392`；Android 模拟器经 `10.0.2.2`
打到宿主的 loopback，所以对它而言这是一个货真价实的远端 agent。

- ✅ **Android TV 整条链路在真 core 上跑通**（TV 模拟器，`android-34;android-tv;arm64-v8a`）：
  名字那一屏 → "Ask to be let in" → 屏上出现 `672448` → `reach` 列表里是**同一个码**、设备名
  `Google sdk_google_atv64_arm64` → `approve` → TV 领走凭证并 `POST /api/session`，core 的设备
  列表把它的 `last_seen_at` 记了下来。**只差最后一步没成**，见下。
- ✅ **三屏都看过**：欢迎页（"Add your agent to put it on this screen."）、名字页（`.hi-agent.xyz`
  贴在输入框后面）、等待页（六位数，across-the-room 大小）。
- ✅ **Android 手机那张 sheet 组合得出来**：文案、名字框 + 后缀、"Scan a QR code instead"、
  "Use a full address" 都在，无崩溃。**但它是在 TV 模拟器上跑的**（手机镜像要 7.4 GB，而 Mac mini
  只剩 6.9 GB，不值得为一张截图把共用机器塞满），所以形状是错的——这不是手机布局的验证。
- ✅ `make android` / `make android-tv` / `make ios` 全绿，引擎 1221 个测试全过。
- ✅ **给名字解析补了单测**（`addressForName`：裸标签进默认 zone、带点/带 scheme 的按整地址走、
  不合法的被拒）。

**🔴 修掉的第三个既有缺陷：遥控器出不了输入框。** `TvField` 的注释原本写着 `singleLine` 就够让
上下键"自由地离开"——不够：`BasicTextField` 自己就吃掉方向键，所以在电视上按下键根本到不了下面的
按钮，键盘能用 `TAB` 跳出去（这就是它一直没被发现的原因：遥控器上没有 TAB 键）。加了
`onPreviewKeyEvent` + `moveFocus`；又发现 Compose 的二维焦点搜索按几何选，输入框横跨整行，于是
下键落在**中间**那颗按钮上（"Hide address"）——按一下名字然后展开地址表单，不是任何人要的。再用
`focusProperties { down = askButton }` 把它指到主操作上。

**纯遥控器路径已实测跑通，全程没有 TAB：**
CENTER（欢迎页）→ CENTER（进输入框）→ 打字 → BACK（只关键盘，不退出）→ DOWN → CENTER
→ core 收到请求，屏上是同一个六位数。等待页还顺手修了一处：进这一屏时主动收起键盘，否则它正好
压住那行"去哪儿放行"的字。

**修掉的另外两个既有缺陷：**

- 🔴 **`make android` 在 `main` 上本来就是红的**。`9f463da` 把地址从 `hi-agent.xyz/ana` 改成
  `ana.hi-agent.xyz` 时改了测试里的字符串，没改**期望的形状**：根地址的 path 就是 `/`，
  `HttpUrl.toString()` 一定带尾斜杠，那条期望改完之后没有任何输入能满足。在基线 commit 上复现过
  才动的手。
- 🔴 **电视上所有标题都是黑字黑底**。`LocalContentColor` 默认是黑色，只有 `Surface` 会改它；
  TV 的屏是 `Box` + `hiCanvas()`（一个背景 modifier，不是 `Surface`），所以凡是没自己传颜色的
  `Text` 都在近黑的底上画黑字。正文传了 `onSurfaceVariant` 所以活下来了，标题没有——"Cores" 和
  旧的 "Pair a core" 一直是看不见的。在 `HiAgentTheme` 里一次性修好。

**还发现并修掉了一个 iOS 也有的显示问题：** 地址栏后缀 `.hi-agent.xyz` 原本一直显示，于是打完整
地址时会读成 `http://10.0.2.2:12392.hi-agent.xyz`——描述了一个根本不会发出的请求。现在只在输入还是
裸标签时才显示，四个客户端加 iOS 一起改。

**没看过的：**

- 🔲 **最后一步：凭证落盘**。TV 模拟器的镜像**不支持锁屏**，keystore 因此没有 user super key，
  `CredentialStore.save` 报 `keystore2: User ECDH key missing / Failed to handle super encryption`。
  这和 iOS 那边 `make ios` 无签名导致的 `-34018` 是同一类事：**环境，不是代码**（`CredentialStore`
  这次没动，旧的配对路径走的是同一个调用）。真机 keystore 正常。
- 🔲 **Android 手机的真实布局**，理由见上。
- 🔲 **Windows 与 Linux 一行都没编译过**。Windows 没有主机（一直如此）；Linux 这次是因为开发机的
  Rust 工具链没了，而 `static.rust-lang.org` 从这台机器上只有 ~27 KB/s，装不回来。两边都是照着已经
  验证过的 iOS/Android 形状写的，但**没有任何编译器看过它们**。

