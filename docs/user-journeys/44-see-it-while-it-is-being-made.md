# 活儿还在做，做出来的东西已经能看了

**Persona:** 派了一件要做一两天的事的人（场地标定：先比一遍现有办法，再把场地线画回样片）。他
不盯过程，但隔一阵会打开首页或看板看一眼这件事到哪了。
**Goal:** 中间一出现有分量的东西——方向站住了、第一版能看了、有个结果只有他的眼睛能判——他打开
就能看到那张图或那段视频，而不是一句"线压在线上"加一个他打不开的路径；也不用专门开口要。
**Preconditions:** 任务在 `doing`，有 worker 在做；worker 手上有图或片子（`work/figures/*.png`、
`out/*.mp4`）。

## Steps & expected UX

1. **worker 做出一张能说明方向的图**，写进展时把它带上：`hi_task_note(update, "场地认出来了……
   线压在线上", attach: ["work/figures/pose_899.png"])`。路径不进句子。工具当场回
   `recorded, carrying att:… (picture 1920×1080)`；文件在那一刻被拷进 `attachments/`，worker 之后
   覆盖这张图、`/tmp` 被清掉，这一行给他看的都还是当时那张。
2. **几秒内首页那张卡下面多一张图**（下一次读台账）。视频的图块右下角写着时长 `0:30`。卡片
   上那一行是句子本身，没有标记、没有 id。
3. **他点那张图** → 这件任务的面板盖在首页上，停在带它的那一行，图在查看器里整张打开，下面是
   那一行原话。Esc 或点外面，查看器收起，面板还在那一行；面板里每一行下面画着它带的图。
4. **视频一样点开就能播、能拖。** 字节带 Range 出，WebKit 也能播。
5. **页面（view）照旧**：`made` 那一行下面画的是页面的截图，点它去那个页面。只是**一张图、一段
   视频不再需要为它做一个播放页**。
6. **他的屏幕不动。** 这些都在看板和首页上——他去看的地方，不是 agent 推到他屏幕上的东西；
   "先别动屏"管不到它。放上屏、在对话里递给他，是 Reaction 的事（showing.md 第 2 期）。

## What must be true

- **句子和证据是同一行。** 附件只能跟着 `hi_task_note` 那一行进来；一行的附件有一个拷不进来，
  这一行就不写——不会留下一句没有证据的结论。
- **内容寻址，不可变。** id 是字节的 SHA-256 前 16 位；同样的字节放两次是一个附件；
  `/api/attachments/{id}` 和它的预览都声明 `immutable`，这句话是真的。
- **代码不从文件夹或句子里推断。** 任务文件夹里有 2,101 张图的也不会被挂出来；句子里写个路径不算
  递交。挂出来的只有 worker 亲手带上的。
- **预览是图块的尺寸。** 960×540 以内，不放大；JPEG，有透明的是 PNG。
- **浏览器放不了的，host 自己转一份能播的。** `mpeg4`（mp4v）这种 Chrome 和 WebKit 都解不了的编码，
  原样收下，后台转一份 H.264（`proxy.v1`）；视频一律从 `/playable` 播——能播的原件、转好的拷贝，
  还在转就让播放器过几秒再问。ffmpeg 跑不起来时，拒的理由是"这台机器的 ffmpeg 跑不起来"，
  不是怪文件。
- **首页不再为了截图去轮询 `/api/views`。** 行自己带着它放上去的东西；只 review 过、从没上过屏的
  页面也会被补拍截图，而不是被丢掉。

## 实测 2026-09-22（本机，scratch data dir，release 构建，**没有模型**）

BYOK 不给 key，所以没有一轮能跑起来——这一趟看的是动词、存储、路由和界面，看不到 worker 自己
决定带不带图。`hi_task_open` / `hi_task_note` 都是直接打 `/mcp`（`X-HI-Role: worker`），素材是
那条真任务里的 `pose_899.png`（2.4 MB）、`mk_450.png` 和 `marked_silent.mp4`（39 MB）的拷贝。

- **图**：`attach: ["work/figures/pose_899.png"]`（相对任务文件夹）→ `recorded, carrying
  att:4ba6b5345f35c157 (picture 1920×1080)`，**39 ms**；记录里是
  `update — …线压在线上 ⟨attached att:4ba6b5345f35c157⟩`。
- **视频**：`marked_silent.mp4` 被拒——编码是 `mpeg4 (Simple Profile)`，浏览器放不了，拒的话里给了
  `libx264` 的转码命令。转成 H.264（9.4 MB）后连同 `mk_450.png` 一起带上，**210 ms**，回
  `clip 1920×1080 · 0:30 · 30 fps` 和 `picture 1920×1080`。路径不存在的那一次整行没写。
- **这台机器 PATH 上第一个 `ffmpeg` 是 x86_64 的**（`~/bin/ffmpeg`，Bad CPU type）。第一次跑视频就
  是被它拒的，而且当时拒的理由写成了"不是图也不是视频"——改成了照实说 ffmpeg 跑不起来；
  之后用产品已有的 `FFMPEG_BIN` 指到 `/opt/homebrew/bin/ffmpeg` 跑。机器本身没动。
- **路由**：`Range: bytes=0-1` → `206`、`content-range: bytes 0-1/9456982`、`accept-ranges: bytes`、
  `immutable`、`nosniff`；预览 960×540 JPEG，三张 96–108 KB（原图 2.4 MB）；不存在的 id、不认识的
  spec 都是 404。`GET /api/tasks` 的这一行 `attached` 按行新旧排好三项，`latest.text` 是去掉标记的句子。
- **headless Chrome 过 CDP**：1920×1080 下首页那张卡旁三张图块、预览都加载了（naturalWidth 960），
  视频块写着 `0:30`；点图块 → 面板 + 查看器，查看器里 1920 宽原图，下面是 13:28 那行原话；Esc 只收
  查看器；面板里两行各自画着带的图；点视频块，`<video>` readyState 4、30 s、1920×1080、无错误。
- **面板排版出过一个错**：图作为第三个 grid 子项占了行头那一列，把句子挤到右边——已改成横跨整行。
- **1512 宽、对话面板开着时首页一张图都没画**：图块的那一栏宽度排在最后分，图块在图表宽度约
  1100 px 以下就会被裁掉（这是 home.md "Pictures last" 的规则，不是这次引入的）。那天他的截图是
  全宽，所以看得到。

### 没看过的

- **worker 会不会带。** general.md 的新段落和 record.md 的提示只在提示词里，没有一个真 worker 在
  有模型的实例上跑过；`make eval-records` 也没按"这一行该带图"重放过。
- **他会不会还要开口要。** 主指标是"给我看看"类消息（基线约 22 天 25 次）；上线后没量过。
- **Reaction 把它放上屏、在对话里递给他**——第 2 期，还不存在。
- **macOS 应用的 WKWebView、手机宽度、电视**里的图块和查看器；远程手机经隧道打开视频（镜像还没做）。
- **手机竖拍的 HEVC、带旋转矩阵的片子**只在解析单测里过过；HDR（HLG）的预览没做色调映射，没看过
  实际颜色。

## 复测 2026-09-22 · 第 2 期（本机，scratch data dir，release 构建，**没有模型**）

素材同上（`pose_899.png`、39 MB 的 `marked_silent.mp4`），都走 `/mcp` 直打。

- **放不了的视频不再被拒**：`marked_silent.mp4`（`mpeg4 (Simple Profile)`）带上用了 **0.30 s**，回
  `clip 1920×1080 · 0:30 · 30 fps · a copy browsers can play is being made`。后台 **1.08 s** 转出
  H.264 1080p 30 s 的拷贝（9.4 MB）；`/playable` 从 `503` 变成 `302 → proxy.v1`，拷贝 `206`、
  `immutable`。图片的 `/playable` 直接 `302` 到原件。
- **放上屏**：`POST /api/views/open {"ref":"att:1b34d1fc630fb3d8"}` 回 `module_url =
  /api/attachments/1b34d1fc630fb3d8/stage.v1.mjs`（三行、`text/javascript`、`immutable`）；列表里
  那张卡 label 是 `Clip`、图是预览。headless Chrome 里舞台上的 `<video>` readyState 4、30 s、
  1920×1080、无错误——那段 mpeg4 原片在浏览器里是放不了的，放的是拷贝。
- **首页和面板换成 `@hi/core` 的组件后照旧**：两张图块预览加载、视频块 `0:30`；点图 → 面板 +
  查看器（这次是 portal 到 body 的那个），原话在下面；Esc 只收查看器；面板里的视频从 `/playable`
  播，readyState 4。
- **`/api/legibility?days=1` 的 `showing`**：`evidence` 读出 2 行、2 行带附件（`update` 1、`delivered` 1）、
  0 行在句子里写路径、到第一份证据 0 h；`show_me` 0——这台实例没有模型，reception 一次都没跑。
- **`hi_say(attach)`**：不存在的 id、传路径而不是 id，都拒并说明；合法的 id 回 `sent`，但对话里什么都
  没有——这台实例没有模型，没有一轮开起来，sequencer 丢掉了轮外的 beat（`unanswered.rs` 已写明的那
  个口子），所以对话里递东西这一半**在这里看不到**。
- **`hi_show(att:)`**：同一个原因，回 `shown`，屏幕没动；上面那次上屏走的是人的"去那里"。

### 仍没看过的（第 2 期）

- **Reaction 在真的一轮里** `hi_show(att:)` 和 `hi_say(attach)`：舞台上它自己放的那次、对话里 agent 递
  的图（气泡里点开查看器、视频就地播）、三条上限按"字 + 每个附件"算。要有模型的实例。
- **reception 真的答 `asks_to_see`**：rubric 改了、字段接上了，没有一次真判过。
- 拷贝在 **HDR（HLG）** 片子上的颜色、长片（十几分钟）的转码时长、两路并发的实际占用。

## 复测 2026-09-23 · 第 3 期（本机，scratch data dir，release 构建，**没有模型**）

这一趟第一次有"外人"：实例同时开了 loopback（12368）和 off-box 监听（12369，绑在 127.0.0.1，
不对外开口），凡是走 12369 的请求都按没有凭据的访客过闸。素材还是那条真任务里的
`pose_899.png`、`court-lines-381…mp4`（H.264）和 `marked_silent.mp4`（mpeg4）。

- **`hi_add_attachment`**：三个文件 **1.13 s** 一起放进去，回三行 `att:… · 文件名 · picture/clip …`；
  id 和前两期一样——内容寻址应该如此。
- **视图里嵌**：手写一个 `court-review/overlay.jsx`，三处 `<Attachment id="att:…" />`，
  `hi_review_view` 渲染出来就是要的样子：图按自己的形状占一栏，两段片子各自带着封面，mpeg4 那段
  放的是拷贝。图在 890 css px 的框里读的是预览，整页宽时读原件（`naturalWidth` 1920）。
- **两个真的坑，都是这一趟才看见的**：
  - `<video controls>` 自己会去要播放器图标（六个 `data:` SVG），还会在拿到头之后取消自己的预加载
    请求（`net::ERR_ABORTED`、`canceled: true`）。原来 review 把它们当错误报，share check 把它们当
    "伸到自己文件外面"——**只要视图里有片子就分享不出去**，review 也会说"没渲染好"。
  - 请求原来只按 path 读，`https://elsewhere.example/views/mine/x.jpg` 会被当成"这个视图自己的文件"
    放行。现在按页面加载的那个 origin 读，别人的机器整条 URL 报出来。
- **分享一个视图**：scope 正好长出它画的那三个 id（`connect_src` 里连 `/playable`、`proxy.v1` 都在），
  别的对象一个都不在。访客用 12369 打开：静态 HTML 里就有内容（`curl` 读得到标题、正文、三个
  `figure`），脚本跑完 React 挂在上面，图升到原件、两段片子 `readyState` 4 / 30 s / 1920，
  **控制台、CSP、加载全干净**。
- **分享一个附件**：`/att/1b34d1fc630fb3d8`，页面就是宿主自己的查看器（黑底、片子就地放拷贝），
  HTML 里是 `<video src=…/playable poster=…preview.v1>`，`og:image` 是预览的绝对地址。
- **闸**：没被任何 share 提到的附件 401；`/api/tasks` 401；未列出的分享没 key 是 401，带 key 200，
  之后子资源走 cookie；撤回之后页面和它带的附件都回到 401。
- **又三个只有跑起来才会看见的**：分享页原来脚本一跑就白——渲染页从 `?module=` 读模块，而分享页
  在自己的地址上，模块现在写在 `<meta>` 里；`connect-src` 原来写的是纯路径，浏览器整条忽略，等于
  `'none'`；没有名字的核上，未列出分享的 key **一次都不打印**，等于发布了一个谁也打不开的东西。
- **视图没有截图就没有 `og:image`**（只 review 过的视图本来就没有截图），发布时补一张。

### 仍没看过的（第 3 期）

- **真 worker 会不会这么用**：`hi_add_attachment` + `<Attachment>` 只有我手写的视图跑过，没有一个
  有模型的 view-builder 按新的 `view-builder.md` 做过一版。
- **真的发给人**：这台核没有名字，链接只在本机；隧道上的分享页、微信/飞书里的链接预览
  （`og:image` 实际被抓）都没看过。
- 第 1、2 期那几条仍然没看过的（Reaction 真的一轮里 `hi_show(att:)` / `hi_say(attach)`、
  reception 真判 `asks_to_see`、HDR、长片转码、WKWebView 和手机宽度）。

## 复测 2026-09-23 · 第 4 期(本机,scratch data dir,release 构建,**没有模型**)

这一期是"每样只留一个":一个浏览器、一张类型表、一个内容键,以及把该删的删掉。

- **一个浏览器**:三次 review 各 **0.25 s**,全程只有一个进程、一个 profile(以前每次渲染都要
  起一个 Chrome 再杀掉)。`kill -9` 掉那个浏览器,下一次渲染 **0.61 s** 自己重起。分享一个视图
  (check 渲染 + 补一张截图)之后仍然只有一个浏览器。
- **但它带出一个新漏**:进程被 `kill -9`(或测试二进制跑完就退)时,那个留着的浏览器活了下来。
  这台机器上十来次 `make test` + 几次 `pkill` 攒了 **27 个孤儿 Chrome**,一个几十到两百多兆。
  现在每个 profile 里写着浏览器自己的 pid,启动时把"生它的那个 hi-agent 已经不在了"的 profile
  连进程一起收掉(只收 `ps` 还认它是浏览器的 pid)。实测:故意留一个孤儿,下一次渲染之后就剩
  一个浏览器、一个 profile。
- **一张类型表**:`/views/probe/clip.mp4` 现在是 `video/mp4`(以前是 `text/plain`——这正是那些
  视图要用 `fetch → blob` 自己搭播放器的原因),`.jsx` 还是 `text/plain`,附件路由不变。
- **一个内容键**:`views/_compiled/` 下是 16 位十六进制的 SHA-256 前缀(`DefaultHasher` 的值,
  编译器升级就可能变,而这个名字是浏览器要缓存一年、分享作用域要点名、外观记录要存的)。
- **删掉的**:`GET /api/tasks/{subject}/files/*` → 404;记录 DTO 里不再有 `files`;面板不再把
  行里的 `` `文件名` `` 变成链接(一行能打开的东西,是它自己带上的附件);`/api/media/drive/…` → 404
  (drive 只由 `/api/drive/file` 服务);`drive/generated/` 和 `store_artifact` 一起删了。
- **生成的东西现在直接是附件**:工具描述、四个提示词、Home 的组图标都改成收 `att:` id。
  **这一条没在有模型的实例上跑过**——这台没有 key,`hi_text_to_image` 一次也没真调起来。

### 仍没看过的(第 4 期)

- **真的画一张**:`hi_text_to_image` → `att:` id → `hi_show`,整条链没跑过(要有 key 的实例)。
- **task-manager 用 `att:` id 当组图标**重画一套。
- 前三期仍没看过的那些(Reaction 真的一轮里 show/递、reception 真判 `asks_to_see`、
  真的把链接发出去、HDR、长片转码、WKWebView 与手机宽度)。

