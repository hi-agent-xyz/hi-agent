# 在手机上翻了一会儿,回到桌面就是那一屏

**Persona:** 同一个用户,同一个 install,两块屏:桌上开着 face,手里还有 iPhone 上的
`WKWebView`。他不认为自己有两块屏,他认为自己有**一块屏**,只是从两个地方看。
**Goal:** 屏幕是一块——agent 摆上来的东西两边都看得见,**人自己翻到哪儿两边也都跟着**;
在手机上看了十分钟放下,坐回桌面,桌面就停在他离开的地方,不需要重新找一遍。
**Preconditions:** cursor 是 appearance 的一部分(`ViewBus`),`POST /api/views/open`
写它,`GET /api/out/view` 送它;trail 是**一条**列表,agent 的 show 和人的 move 都往里
追加,每条记着是谁的手。见 [`stage.md` § One screen, and the cursor is on it](../arch/stage.md)。
**与 [20](20-reuse-built-views.md)(翻工具箱复用)、[32](32-quick-views.md)(快出一屏)相连。**

---

## Steps & expected UX

1. **手机上点开 `factory/drive`** → 桌面那块屏**当场跟过去**,不需要刷新、不需要问。
   桌面上停着的那次长轮询直接醒来,拿到 `cursor`。
2. **agent 这时 show 了别的东西** → 两块屏都被带走。「一个 show 带走每一扇窗」现在只写
   一次:show 追加一张卡并把 cursor 放下。
3. **回头翻 band 里的旧卡** → cursor 移动,**行不重排**。手指正在划的那一行不能在手底下
   跳。只有**头一次**到一个地方才发一张新卡。
4. **放下手机,坐回桌面** → 桌面就是他离开时那一屏。刷新也一样:重新加载读的是同一个
   cursor,而不是 agent 最后一次 show。
5. **「把屏幕还我」** → slot 和 cursor 一起清掉,回到空房间。以前这是两次写,两次可以
   互相不同意:slot 清了,窗口手上停着的旧 view 还在。
6. **人翻到哪儿,agent 知道** → 下一轮 context 里直接说「他们把屏幕带到了 X」,不带年龄、
   不带需要权衡的口气。服务器自己拿着 cursor,它对自己不会过期。

## Expected outcome

- 一块屏,两只手写:agent 的 `hi_show` 写 slot,人的 `views/open` 写 cursor,同一个
  version、同一条长轮询,没有第二条会跟第一条吵架的同步路径。
- 窗口自己**不再留副本**:滚动位置、对话是抽屉还是整屏、皮肤、frame 还是各窗自己的。
- 「这个数字不对」问的是他眼前那块板,不是 agent 最后摆上去的那块。

## Edge cases & failure modes

- **人在桌面前,顺手戳了下手机** → 桌面动了。这是**认下的代价**,和 show 本来就会对他
  做的事一样,只是这次是他自己的手。
- **cursor 指向的卡被 `HISTORY_MAX` 挤掉** → 回到 live,而不是渲染一个到不了的地方。
- **重启** → **整行都在**;cursor 落在最后一次「到达」上,可能不是最后一次「翻到」。

---

_机制:`Appearance` 多一个 `cursor` + 每条 history 记 `Hand`(show / move);
`POST /api/views/open` 是人这一边唯一的写入口(ref / module / live 三种说法);
`POST /api/in/view`、`Attention`、以及围着它的一整套「会过期,所以要报年龄」都删掉了——
服务器自己拿着的事实对自己过不了期。到达写快照,行内走动不写。成熟度:**built + 实测**。_

## 实测 2026-09-01 · design/one-screen(基于 origin/main 332fed9)

Mac mini,独立 `--data-dir /tmp/os-dd`,端口 12414;两个 `GET /api/out/view` 当作两扇窗。

- ✅ **一扇窗动,另一扇跟**:B 停在 `?since=0`,A 发 `open {"ref":"factory/drive"}` →
  B 当场醒来,拿到 `cursor: "factory/drive"`;`views` 仍是 agent 的 slot,没被人改写。
- ✅ **刷新落在 cursor 上**:全新一次 `GET`(等于重新加载)读到同一个 cursor。
- ✅ **到达发卡,回头不重排**:开 drive → tasks → memories,行是 `[Drive, Tasks,
  Memories]`;再回 drive,cursor 变了,**行一模一样**。
- ✅ **回 live / 到不了的地方**:`{"live":true}` → cursor 清空,屏幕还在;`{}` → 400。
- ✅ **重启后整行还在**:人自己走出来的三个地方 + cursor,`kill` 再起来照旧。
- ⚠️ **只在 wire 上看过,没在浏览器里看过**:两扇窗是两条 curl,不是两块真屏幕。
  「show 带走每一扇窗」这条走的是 `make test` 里的集成用例(真 seam、真 HTTP),不是
  真 model —— 这台机器上 codex 起不来(`failed to initialize sqlite state runtime`)。
  band 里点卡片、手机上真的跟过去,仍待在有桌面会话的机器上复测。

### 这次实测改掉的设计

第一版**两半都不写快照**,理由是 `raw/appearance/` 是「agent 表达过什么」的档案。对
cursor 成立,对**行**不成立:在一个 agent 还没 show 过任何东西的 core 上,从来没有写过
快照——重启拿走的不是一个近似的 cursor,而是**他去过的每一个地方**。读代码看不出来,跑
一遍就看见了。现在**到达写、行内走动不写**。

## 实测 2026-09-07 · view 被改写时,停在上面的那一屏跟不跟

**先是在用户自己那台开发机上撞见的缺口。**人停在
`knq-project-architecture/editing-live-commentary-overview`(cursor 就是它),builder 在
12:03、12:04 两次改写它的 `.jsx`,屏幕却一直挂着 **11:56 编译出来的那份**
(`ff8268e5a31feb52.mjs`)。12:03 那次连新产物都已经在盘上了(`01286aebc1d45cac.mjs`,
`hi_review_view` 编的),屏幕没有去取。用户的原话是「要切到另一个 view 再切回来才看得到新的」
—— 切回来走的正是 `POST /api/views/open`,它按 ref 重新 resolve + 编译,所以那是个绕法,
不是设计。

原因在这条线上是清楚的:module URL 是**上屏那一刻**源码的内容哈希,`apply`(show)和
`go_to`(open)各自把它钉死;而 view 是**直接写文件**存的(`view-builder.md`:「no
special tool, just write the file」),没有任何一个工具调用可以挂钩。`refresh_sources`
只说了开机那一刻的规则。

**修法与复测**(`view_watch.rs`;独立 `--data-dir`,端口 12401,release 二进制):

- ✅ **改写就跟上**:`open {"ref":"factory/drive"}` 停住,一条 `?since=4` 的 long-poll 挂着;
  往 `factory/drive.jsx` 追加一行 → poll **当场返回**,card 的 module 从
  `fe8b448afbefe02f` 变成 `91d080e32e92706a`,cursor 不动、id 不变(所以是换模块不是重挂槽)。
- ✅ **没人看的 view 改了不动屏**:同时改 `factory/tasks.jsx`,version 停在 5 —— 它在行里,
  但下次被打开时自然会重新 resolve。
- ✅ **存了个编不过的中间态**:往正在看的 `drive.jsx` 里塞坏语法 → version 不动,屏幕留着
  上一份好的。这是 `refresh_sources` 同一句话:陈旧的 view 也好过空房间。
- ✅ **改回来又跟上**:修好文件 → version 6,新 module。
- ⚠️ **只测了「人停在上面」这一半**。agent 自己 content slot 里那一半(`apply` 写的)要一次
  真的 `hi_show` 才能在活实例上摆出来,这次没跑;它只有单测
  (`a_rewrite_reaches_the_view_the_agent_has_up`)。
- ⚠️ **没在浏览器里看过**。看的是 wire 上的 module_url 变了;`ViewMount` 的 `[moduleUrl]`
  依赖会重新 import,这一步是读代码推的,不是看见的。

## 实测 2026-09-07 · band 里的缩略图是谁那张脸的图

**又是在用户自己那台开发机上撞见的,还是同一个 view。**人在一扇 1920×1050 的桌面窗口里看
`knq-project-architecture/editing-live-commentary-overview`,band 的 history 第一格却是一张
390 宽的手机页面 —— 160px 的格子里 17px 正文还读得清,因为底图根本不是这扇窗的排版。

盘上的证据是齐的:`data/views/_shots/ref/` 里,桌面拍的图一律 `479x262`(= 1920×1050 ÷ 4,
`THUMB_WIDTH` 的长边上限),而 10:24 之后 12:11、12:48、12:59 三张全是 `393x852` —— iPhone
的竖屏视口。当天 50 张具名 surface 的图里有 13 张是手机形状的。

原因在 `view_render::stage_frame()`:它取 `STAGE.first()`,也就是**最近一次上报**的 surface。
而上报是边沿触发的(resize、主题翻转、页面加载),所以一扇有人正在读、但尺寸没动的桌面窗口
永远抢不回队首,手机开一次揣兜里就一直是 primary。

**修法与复测**(release 二进制,独立 `--data-dir`,端口 12360;桌面 1920×1050 先报、手机
393×852 后报,所以手机是 primary):

- ✅ **桌面开 band,拿到的是桌面的图**:`GET /api/views` 带 `X-HI-Face: deskface`,warm 出来的
  十张全部 `479x262` —— primary 仍然是手机。这条在 main 上会是十张 `393x852`。
- ✅ **手机开 band,同样十张被重拍**:带 `X-HI-Face: phoneface` 再读,十张全部变成 `393x852`。
  当时每张图只有几分钟大、源码一个字没动,所以走的确实是新加的形状判断 —— 在 main 上
  `good_enough` 会把它们全留下。
- ⚠️ **只测了 band 的 inventory 那条路**。`POST /api/views/open` 也带了 `X-HI-Face`,那半是
  单测(`a_picture_taken_for_another_face_is_re_taken`)+ 读代码,没在活实例上点过。
- ⚠️ **没在浏览器里看过格子**。看的是盘上 PNG 的宽高,band 里那张图实际长什么样是推的。
- ⚠️ **两张脸同时开着 band 会互相重拍**。设计里按 accepted 写下了(三张一轮、一次一个浏览器),
  没去实测它到底有多吵 —— 见下一节,量出来了,而且这条设计整个被推翻了。

## 实测 2026-09-08 · 两张脸同时开着 band,到底有多吵

上一节最后那条 ⚠️ 的答案:**大约每秒一次,而且吵的不是 band,是这台 core 上挂着的每一扇窗。**

还是用户自己那台开发机撞见的。`make dev` 的日志里 `GET /api/out/view long-poll opened` 一秒滚
两三行,dock 上 Chrome 的图标一直在弹。在活实例上量的:

- 30 秒里 appearance 版本 `4256 → 4279`(+23),同一段时间 `data/views/_shots/ref/` 下正好写了
  23 个 PNG —— 一比一,版本是被缩略图顶着涨的。
- 这 23 次重拍里 18 次宽高比直接翻转,`393x852 ⇄ 479x271` 来回。`/api/surfaces` 里 iPhone 的
  `last_seen_at` 就是当时,桌面窗口也开着 band,两边都在每 3 秒读一次 `GET /api/views`
  (`INVENTORY_POLL_MS`)。
- 起的是系统里那个真的 Chrome:`/Applications/Google Chrome.app/Contents/MacOS/Google Chrome
  --headless --user-data-dir=…/hi-render-<pid>-…`,平均 1.3 秒一个。

**accepted 那条把账算漏了一项。**它算的是浏览器(三张一轮、一次一个),没算 `note_shot`:每张
图落地都 bump appearance 版本,而那是所有窗口同步屏幕的那根线。所以两张脸争一张缩略图,唤醒的
是每一个挂着的客户端,每次一条日志。band 内部有界,跨 install 无界。

**修法不是把架调停,是把架的前提删掉。**第一版改成按形状分文件存(bucket 正好一个容差宽,
同 bucket 互相认、跨 bucket 不碰面),量下来确实归零了 —— 但那是为一个**在 118×76 里根本看不出来
的区别**付一份 (view × 形状) 的渲染 + 一路穿透的 face 参数。9-07 那次真正看见的坏处是**竖图落进
16:9 的格子变成一条窄条**,那是宽高比,不是排版。所以最后落的是:**所有缩略图固定按格子自己的
1280×720 渲**,盘上永远 480×270,谁读都一样。详见 `docs/arch/stage.md` §
*A thumbnail is rendered at the tile's own frame*。

**复测**(release 二进制,独立 `--data-dir`,端口 12360;`POST /api/stage` 报两张脸:`phone`
393×852、`desk` 1512×856,两张脸各每 3 秒读一次 `GET /api/views`):

- ✅ **预热一轮就停**:10 个 factory 具名 view 各拍一次,共 10 张 —— 不是 20 张,两张脸共用。
- ✅ **稳态是零**:两张脸继续轮询 60 秒,appearance 版本不动,新写的 PNG 0 张。同样条件在 main
  上是 30 秒 23 张。
- ✅ **盘上全是 `480x270`**:跟格子的 16:9 对齐,不再有竖条。
- ⚠️ **没在浏览器里看过格子**。两张脸是 `POST /api/stage` 报的,band 的读是 curl 打的;真的两台
  设备同时开着 band 没试过,格子里那张图长什么样也是从盘上 PNG 推的。
- ⚠️ **旧图的自愈只有单测见过**。复测用的 `--data-dir` 是新的,所以"旧的 393×852 被读到时重拍成
  480×270"这条走的是 `a_picture_of_any_other_shape_is_re_taken_once`,没在有历史数据的实例上跑过。

### 顺带发现:手机上的 view 从来没被渲染过

`view_render::surfaces()` 的注释说它是给 "the refine pass ... to render the frames the first
show deliberately skipped" 读的。**它没有任何生产调用者,只有测试** —— 那个补渲其他 frame 的
refine pass 不存在。而 `hi_review_view` 不带 width/height 时渲的是 `stage_frame()`,也就是最后
上报的那张脸;`src/identity/workers/view-builder.md` 又明确劝阻 builder 自己挑宽度("the most
expensive habit in this loop")。

合起来:人在桌面上跟 agent 说话 → builder 在桌面 frame 上审 → 通过 → 发布。**手机版排版在发布
前一次都没有被渲染出来看过**,等人在手机上打开才是它第一次以那个宽度存在。用户报的"mobile 上
排版常常不尽人意、各种换行"很可能就是这条链路的直接后果。**未验证** —— 还没在 393px 下真渲过
现有的 view 看它到底怎么坏。


## 实测 2026-09-08 · 手机横屏为什么不像桌面,以及 view 一直在问错对象

用户报的是"手机上排版挤压很厉害,横屏也挤,希望横屏能接近桌面"。拿他截图的那个 view
(`sports-ai-industry-2026/sports-ai-streaming-commentary-radar`,编译出来 hash 一致)在三个
frame 下真渲了一遍:

- **竖屏 393×852**:`h1` 被 `@media (max-width:480px)` 压到 46px,8 个汉字占满一整行;正文
  68 处硬写 `font-size:Npx`、只有 3 处 `clamp()`。**它是被看过的** —— 232 个 builder/reviewer
  session、2283 次 `hi_review_view` 里,**390px 是出现最多的宽度,904 次,占 40%**。所以竖屏差不是
  "没人看过",是看了 904 次然后接受了。
- **横屏 852×393**:第一屏只有半个标题。852 比这个 view 的 820 断点大 32px,掉进平板档,
  `h1` = `7.4vw × 852` = 63px,配 393px 的高度。**2283 次调用里 852px 出现 0 次。**
- 60 个 agent 写的 view 里 46 个有 `@media`,70 个断点值散在 390–1180 之间,**852 高于其中 33 个、
  低于 37 个** —— 转个屏幕拿到哪套排布,每个 view 抛一次硬币。

### 根因不是断点写错,是 view 问错了对象

`ViewSlot.tsx` 一直写着 "Every view owns the frame it is handed",但 view 用 `@media` 和 `vw`
问的是**浏览器窗口**。两者只是经常碰巧相等:popover 里槽是 ~380px,手机横屏是 852px,面板打开时
是 432px。`shape.ts` 记的那个"popover 的 max-width 把 channel disc 压到 32px"的老 bug,是同一件事
从宿主那侧发生了一次。

**修法:槽声明成容器,view 问槽。** `container-type: inline-size; container-name: hi-view`,
契约写在 `view-builder.md`(`@container hi-view` + `cqi`)。这样宿主才第一次有了一个可以拧的旋钮:
把太小的房间**报成 1280**(`view_render` 的 `DEFAULT_WIDTH`),再 `zoom` 画到贴合玻璃。

**为什么不是改视口**(浏览器"请求桌面版"那条路):实测 `zoom` 只搬动布局盒子,**`@media` 和 `vw`
纹丝不动** —— 852 窗口、0.666 因子下,槽布局宽 1280、`@container (min-width:1121px)` 命中、
`10cqi` = 128px,而 `@media` 仍报 820–1120 档、`10vw` 仍是 85.2px。改视口能同时搬动那两样,但它
会把宿主自己的控件一起缩(44pt 变 29pt,这个 face 已经犯过一次),而且**它会改掉自己刚量到的那个宽度**。
缩槽两样都不碰。

### 复测(Chrome DevTools 设备模拟,852×393 @3x + 触摸,release 二进制,独立 `--data-dir`)

写了一个纯 container-query 的探针 view (`probe/room`) 来验证整条链:

- ✅ **面板收起(`room` 停位),槽 = 852**:`data-view-zoom` 置位,factor `0.665625`,
  **view 读到 1280、画出来 852**,`@container (min-width:1000px)` 命中 → 四列网格 + 大标题,
  一屏装完。就是桌面的排布,落在手机真实的 2556×1179 像素上。
- ✅ **面板并排(`panel` 停位),槽 = 432**:**不缩放**,view 老老实实按 432 排一列。
  0.34 的因子低于 `MIN_SCALE`,被地板挡掉了。
- ✅ 这条地板同时也是把竖屏手机(393px → 0.31)挡在外面的那条规则 —— 一条规则,不是两条。

### 这次改掉的一个设计错误

第一版从 `window.innerWidth` 算因子。实测打脸:**窗口 852,槽只有 432**(面板占 420),于是它按
0.666 缩了一个半宽的槽,view 读到 649 —— 两个数都不是。所以因子改成**量槽自己**
(`ResizeObserver`),`resize` 监听器根本看不见面板开合。

### 未验证 / 待定

- ⚠️ **没在真机上看过。** 设备模拟是 Chrome 的,WebKit 没跑过。最终得在 iPhone 上验一眼。
- ⚠️ **老的 60 个 view 不会变好** —— 它们写的是 `@media`/`vw`。这是明确接受的取舍。
- ❓ **手机横屏时面板应不应该并排?** 现在 `shape.ts` 判定横屏手机是 `wide`,于是面板成了 420px
  的侧栏,view 只剩 432 —— 比竖屏的整屏还差。用户想要的"横屏接近桌面"在面板并排时拿不到。
  这是 `stops(shape)` 的问题,不是这次改动的范围,单独记在这里。
