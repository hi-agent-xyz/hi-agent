# User Journeys

A living catalog of concrete cases and their **expected UX**. Each file documents
one journey: what the user is trying to do, what they see and do step by step, and
what the agent/system is expected to do in response.

These are the source of truth for *intended* behavior — write the expected UX here
first, then build/verify against it. When behavior and a journey disagree, that's a
bug in one or the other; resolve it explicitly rather than silently.

**Gaps found by running these against a live instance live in [gaps.md](gaps.md)** —
one entry per problem, with the evidence that was verified *outside* the conversation.
Each journey file keeps its own `实测` section for what one run looked like; `gaps.md`
carries the cross-journey issue itself, so a problem is written down once.

## How to add a journey

1. Copy the structure below into a new file: `NN-short-slug.md` (e.g. `01-first-launch.md`).
2. Keep it concrete — real clicks, real screens, real messages, not abstractions.
3. Describe expected UX, not implementation. Link to architecture/code only when it clarifies.

## Template

```markdown
# <Journey title>

**Persona:** who is doing this (and what they already know)
**Goal:** what they want to accomplish
**Preconditions:** what must be true before this starts

## Steps & expected UX

1. **User does X** → system/agent responds with Y (what they see, hear, feel).
2. ...

## Expected outcome

What "done" looks like, and how the user knows it worked.

## Edge cases & failure modes

- What happens when <thing goes wrong> → expected handling.

## Open questions

- Anything undecided about the intended UX.
```

## Index

- [01 · 羽毛球男单世界前十](01-badminton-top10.md) — 打招呼 → 异步检索演示前十 → 钻取单个球员 → 切换到天气;确立对话简短、音画结合演示、窗口式轮播、柔和转场等通用原则。
- [02 · 飞书群消息 → Sprint-Backlog 任务](02-feishu-sprint-backlog.md) — 常驻委托,从零开始:置备工具(装 CLI、建应用、鉴权,老板只做必须的事)→ 对齐 → 试运行 → 长期值守;确立缺工具是任务的一部分、向上沟通最小化、heartbeat 自我注意、重启自恢复等原则。
- [03 · 飞书群 flash-cards → 记忆卡片图](03-feishu-flash-cards.md) — **按真实运行写成的完整实例**:委托 → CLI 置备与三次扫码 → 样稿校准(翻车与重做)→ 补齐存量 → 断后自愈;确立交付必检、长活不占线、完工交差、自己的摊子自己管;附实测缺口清单。
- [04 · 看 GitHub/小红书/抖音在火什么](04-trending-feeds.md) — 即时世界态·按需现查(扩展 01);内容不入持久记忆,只有站定兴趣才落 facet。
- [05 · 今天有什么大新闻 / 盯住油价](05-news-and-watch.md) — 一次性现查 vs "盯着"落成长期关注 + pulse 主动浮现;重启不丢盯。
- [06 · 我附近的羽毛球活动](06-badminton-near-me.md) — 01 的延伸:用站定兴趣(羽毛球)+ 位置过滤 + 主动浮现一句,可叫停。
- [07 · 用浏览器替我办事](07-browser-errand.md) — 浏览器 effector 实操(非纸上谈兵);敏感动作停下请示;怎么开页沉淀成技能。
- [08 · 操作电脑/手机上的应用](08-operate-apps.md) — Mac/Win/Linux/Android/iOS 的可行性光谱;有句柄才做,没有就诚实说清。
- [09 · 用微信(诚实面对脆弱面)](09-wechat.md) — 个人号无开放 API + 反自动化;给受限路径,不假装与飞书同等。
- [10 · 用 SAM/YOLO 做视觉活](10-vision-sam-yolo.md) — **测的是"能建项目 + 接上 appearance"**(视觉任务只是载体,非通用视觉感官):真跑出检测/掩膜并落 `views/` 呈现;首选不灵主动换方案(连 14)。
- [11 · 在中国报个税](11-china-tax.md) — 半稳定领域:用前现查当年政策/数字;截止日可作主动提醒。
- [12 · 陪孩子:讲故事/教认数字/做图](12-play-with-child.md) — register 适配 + 适龄 + 安全边界;够精致的图走 views/。
- [13 · 配一个外部能力(API + 凭证)](13-equip-a-capability.md) — 能力分流:认识进记忆/技能,凭证逐字进 drive 笔记本,密钥不进脑子。
- [14 · 你对 YOLO 的了解随用而长、被实践修正](14-knowledge-grows.md) — competence 读自证据图(不存等级)+ provenance + 先验被 lived 超越;验证知识模型的核心 journey。
- [15 · 打断:我一开口,它就让路](15-talk-over-the-agent.md) — 语音对话的底座:嘴串行、字跟嘴走不抢跑;我插话它当场停声并清掉没说出口的尾巴,下一轮带"说到哪 / 我说了啥"重新组织,不复读。
- [16 · 先认得脸/声音,后来才知道名字,然后处处用得上](16-recognize-people.md) — 身份=生物特征簇:不报名字也能先把人记成"同一个人"(mint 一个 id),名字从对话里学到后把 id 改名成名字;认人是软证据、容忍模糊、可纠错。脸已端到端实测(buffalo_l),声音待建。
- [17 · 播放音乐(开 app → 搜放 → 投屏 → 记住偏好)](17-play-music.md) — 随口点歌:有 app 就用、没有先请示装;搜到真放出来;播放界面投成 view,可收起转后台(收画面≠停播);第二次记住用哪个 app、怎么搜放、投屏与否,更省事。
- [18 · 我要传你点东西,怎么弄(摆出上传入口:拖拽区 + 二维码)](18-send-files-to-agent.md) — 最基础常用的一步:把东西递给它。优先**直接摆两个 view**(拖拽区 + 手机扫码上传页)而非口述选项;入口绑 conversation。文件 = 递来的物件按引用,不走 vision 感官。行为靠 show 已有,carrier(上传端点 + 手机页 + 二维码)未建。
- [19 · 直接传一张护照照片(收下 → 看懂 → 存进 drive → 妥帖回话)](19-upload-passport.md) — 把文件当**物件**收下:原件逐字进 drive、认识带出处进脑子(不是当"看到一幅画"配字幕);看懂是什么/属于谁,妥帖回话,敏感件确认意图、默认私密;日后"我要护照"找得回。carrier/drive/解析未建。
- [20 · 重复用到的 view 越用越快(手上还有就直接 show / 其余翻工具箱)](20-reuse-built-views.md) — 像人用工具箱:ref 还在手上→直接 `hi_show(ref)`;其余一律 delegate,由 builder `grep "^// purpose:"` 翻全树,翻到就改旧件、翻不到才从头。每个 view 第一行自带用途,所以"索引"永远从树上扫出来、不另立第二真相源;意图没随 brief 传到时**默认重做**(给旧快照是错,重做只是慢)。跨 session 常驻索引 **2026-08-08 明确不做**;参数化 view 待建。
- [21 · 把一坨数据交给它(Apple Health 导出 / Claude Code 会话)](21-hand-over-bulk-data.md) — 大宗/结构化数据不是 ETL"导入":先落 raw(落即 precious、不丢),值得留的逐字进 drive、能理解的化进 facets。两扇门:明确"存好"→ live 当场委托 worker;"发现值得留"→ reflection 像 view 那样毕业。工作记录贴近 episodes/facets;量化时序留 drive + 交独立 apple-health skill,不硬塞记忆。坑:大字节别穿 raw 再复制进 drive(两棵都 synced)。drive/毕业/增量合并待建。

> **像人一样攒能力的三条反射(22–24)**:不是塞一个大知识库,而是按 decay-rate 配上**获取反射** + 元心态。研究反射(开工前别拿陈货)→ 批判反射(收工前别交 dumb)→ 技能沉淀(把贵经历存成起点,且会重新核当下)。研究/批判主体是 soft-guidance;技能沉淀要一个 `skills/` 工坊 + reflection 多策展一类。

- [22 · 给我剪个集锦(研究反射:别拿陈货当数,先去看)](22-research-before-stale-answer.md) — 把"现查"从用户明说的易例([04](04-trending-feeds.md)/[11](11-china-tax.md))推到 agent **自以为知道**的难例。触发是个**味道**:要给"best/latest/现在/哪个工具/什么流行"的判断时——"我不是**知道**、是**记得**"→去看(含**看范例**校准品味,不只查事实);限定**快过期层**(durable 手艺不查),否则每件小事都查会卡。能力本就有(worker 有 web+code-exec),缺的是**反射触发**——三条里最便宜、该先落。**已实测通过**(它现查 Kokoro 而非背旧榜)。
- [23 · 剪完先自己看一遍(批判反射:好看,不只是能用)](23-critique-before-shipping.md) — 交付前**冷眼自评**,对着研究反射现查来的好范例打分:"能用"≠"好",不到位再来一版、**过线即止**(非无限磨)。把 core.md 的对不对自检(succeeded≠right)推到**好不好**,直接堵 works-but-dumb;在 worker 现有"work to completion"偏向前加一道好不好闸。实测:worker 渲染成图、看实际成品再迭代;审美过线闸未单独隔离。
- [24 · 第二次剪快得多,而且没用陈货(技能沉淀:难活变顺手流程,且重新核当下)](24-skill-improves-and-refreshes.md) — 一次又查又试又翻车的贵经历沉成 `skills/` 一条笔记,第二次**从 bar 起步**、明显快;但**技能=起点非真理**:durable 半(我怎么干)稳用、transient 半(当下什么好/哪个工具)每次被研究反射([22](22-research-before-stale-answer.md))**重核**——正面解 [11](11-china-tax.md) 的"技能别把旧数字焊死"。门槛=难+会再来+干成了;reflection 策展(照它策展 facets/drive)。实测:worker 自己写了一条技能笔记(contribute 路径通过);reuse/策展/transient 标记待复测。
- [25 · 干到一半被打断,重启后自己接着干完(一次性交付的断点恢复)](25-resume-interrupted-work.md) — [03](03-feishu-flash-cards.md)/[02](02-feishu-sprint-backlog.md)/[05](05-news-and-watch.md) 常驻职责自愈的**孪生**:恢复的不是"让监听活着",而是**做到一半的一次性交付**(欠老板的那几张卡)。半成品交付 = 一条**临时承诺**,接活当下记进 **task ledger**(`memory/facets/tasks/`)、交付即划掉;重启后读记忆-醒来-注意这个**既有回路**注意到没划掉的 loop → 先看已落什么(不重做副作用)→ 面向用户出声浮现 / 内部悄悄补完。reflection 兜 jot-before-crash 窗口("答应了却没见交付" → open task)。SHIPPED 2026-06-25(57a757c),built+green,**未实测**。

> **通用视觉感官(26–27)**:把"看懂"作为一路感官接进来——不是建 CV 项目([10](10-vision-sam-yolo.md))、不是认人([16](16-recognize-people.md))、也不是收文件([18](18-send-files-to-agent.md)/[19](19-upload-passport.md)),而是 agent 自己看懂一帧 / 一段并化进记忆。两个端点:**frame**(image+prompt→文字)管"一刻",**video**(video+prompt→文字)管"一段"。今天只有 [08](08-operate-apps.md) 用到通用视觉(see-to-act),这两条补上 see-to-answer / see-to-remember / see-an-event。

- [26 · 看懂一帧:举起来当场问 / 存下来回头找](26-look-and-recall.md) — 通用视觉**最典型一路**(frame endpoint):举起实物 / 发来照片,既当场答到点(剂量、成分、跨设备读报错),这份"看懂"又留存让图按**内容**找回;升级现有固定字幕路径(`server/vision.rs`)。与 [16](16-recognize-people.md)(脸=内置模型软证据)、[19](19-upload-passport.md)(文件=物件不走感官)区分。
- [27 · 看我做,给我反馈(看一段过程,不是一帧)](27-watch-and-guide.md) — 通用视觉 **video endpoint**:看懂一段过程的先后 / 节奏 / 对错(发球、做菜),给针对性反馈、跨段对比进步;语气陪练式(连 [12](12-play-with-child.md))。区别于 [10](10-vision-sam-yolo.md)(建项目跑 CV 模型)。
- [28 · 第一次跟它说 "hi"(开箱第一面:欢迎 view + 自我介绍,然后让位)](28-first-meeting.md) — OOTB 第一印象:全新 install(seed 据空 history 判"第一次见")时,一句温暖简短的自我介绍——你就跟我说话一起干活、我记得你、我能用你的工具、而且**能被教会**学一次就会——边说边摆预置内置 view `_builtin/welcome`,然后**收住让位**。不是导览 / 向导 / 教学,只落印象和几条核心念头;**只此一次**(有 history 后提示自清)。欢迎 view 是预置 seed(像 `_builtin/upload`),"不预置资产"的有意例外。net-new 已写待实测。

> **测试即素材(29)**:把"体验一个产品 + 传播"整件交出去——agent 在能碰到的设备上真操作、录下来、剪成对外成片、操作登录态 app 发出去。缝合已有几路(操作设备 / 浏览器 / 看一段过程 / 研究+批判反射);盘完**核心回路无硬缺口**,都是驱动已有 effector。

- [29 · 去体验个产品的新功能,剪条小红书发出来(测试即素材)](29-test-and-post.md) — 整件"体验 + 传播"交出去:在能碰到的设备(Mac mini)上真操作 pi.dev 新功能、边测边录、**判定跑没跑通(红的落 bug 报告不发)**、干净那次剪成成片(裁/字幕/旁白/竖版)、**发布前过目**、拿到 go 后**操作登录态的小红书**发出去(**非 API**)。缝合 [08](08-operate-apps.md)/[07](07-browser-errand.md)/[27](27-watch-and-guide.md)/[22](22-research-before-stale-answer.md)/[23](23-critique-before-shipping.md);核心回路无硬缺口(设备接口 abacad 式 MCP + 设备自带录屏/ffmpeg + TTS-to-file + 操作登录态 app),唯一待确认原语 = **TTS 渲文件**,待打通 = Mac mini 录屏 TCC。net-new 未建未测。

- [30 · 画张图,再改一版(生成即产物,存进 drive 拿 ref)](30-make-a-picture.md) — 四个生成任务(text/image-to-image、text/image-to-video)从"声明了但没接线"到真跑通:**模型由 agent 挑**(工具描述现列当下可达的模型 + 最好/最快/最便宜),旋钮咽不下就报错点名换谁能办、**绝不静默丢弃或换模型**;产物落 `drive/generated/`(**不褪色**,区别于 raw 的相机帧),`drive/<path>` 作为 ref 语法第二条臂,让相机拍的 / 别人递的 / 自己刚画的走同一个参数;视频不占线,几分钟后以**一条消息**带 ref 送回。填上 [12](12-play-with-child.md) 实测记的"图是代码画的、非图像模型"。**图像那条已实测出图**(2048×2048 真图落 drive、ref 可回喂、不能编辑的模型如实拒绝);gpt-image-2 只有单测、视频两条未跑;agent 自主派单那段被本机 Reaction 不开口挡住,待复测。
- [31 · 听见整个房间,但只在该接话时接话](31-hear-the-room.md) — ASR/diarization 只做机械感知,Reaction 从整段语义判断话是不是在跟自己说;旁聊、电视、问句碎片不触发回答或任务,却可作为近期外围语境,在后来被问到时重新关联。身份只说明谁说的,不说明说给谁;不依赖 wake word。指南已补,运行时 sender/外围上下文/cognition handoff 待实现与实测。
- [32 · 基础信息先给一个 Quick View,复杂了再认真做](32-quick-views.md) — **软引导**而非 DSL,但这条线是**机械可判**的:整页只有一次排布(单元素 / 一个 flex / 一个 grid)、组件只出自 quick set(呈现型的十二个;藏内容的 `Tabs`/`Accordion` 和收输入的表单类不在其中)——**先列 import** 就能判。**判断属于 builder 不属于 agent**(agent 只有用户的话,builder 才有材料),拿不准按 Quick 走,太小了就地升级(同文件同 ref)。Quick View 有自己走得完的流程,构图分类 / rough-then-refine / refine pass / 表现力标准都只属于 Custom;真实渲染、主题、窄屏、空态和错误态仍是两条路共同的底线。原先这条线只是一段"看情况"的判断,后面跟着七百行 Custom 流程,所以**几乎从不发生**。
- [33 · 活儿在你忙着别的事的时候干完了(它照样送到你手上)](33-work-finishes-while-you-are-busy.md) — 交付的最后一米:**产出存在 ≠ 人拿到了**。worker 说的 delivered 是「交到你手上」;Cognition 坐在对话之外、发出去的都是提议、没有回执,判断交付**天生是瞎的**——所以补给它 `## On their screen`(真正上过屏的 view),而不是给它一道闸(那条路建了又拆:靠 agent 记得填的字段去强制它记得做事,忘了填时是**静悄悄**地不在)。人在等的那件事在一条多主题消息里**先说**;现场太热可以压着,但那是个**带期限**的决定(`back_in`),不是沉默;账在**人看见之后**才关,兜底是下一轮再递一次。与 [25](25-resume-interrupted-work.md) 孪生(那条是被重启打断,这条是干完了没送到)。提炼自 2026-08-18 真实翻车([gaps#29](gaps.md));已建 `8f09542`,**未复测**。
- [34 · 你还没开口,下一步已经备好了(该停的那步停在门口)](34-a-step-ahead.md) — 交付那一刻是信息量最大的一刻,而在此之前那一刻什么都没花在「接下来会发生什么」上,于是人等两次:一次等交付,一次等交付之后那件显而易见的事。两件:交付**自带它会引出的问题的答案**(交出去之后他说的下一句如果你本来就答得上来,那它该在板上);同一轮里派一个 `ahead: true` 的 worker 去做下一步里**可撤销、外面看不见**的那半。边界不是新的 —— 就是单独行动时那条(可撤销且外面看不见就做,单向或留痕就停在门口),早一步不是放宽的理由。备好的东西不占 floor、不通报、过期重做、没人要就无声作废。测法是**造情境别提要求**,而**没等 go 就发出去**那一条不过则全部不算。已建(`docs/arch/agents.md` *Working ahead* + 三份 prompt + `ahead` 计数),**未实测**。
- [35 · 在手机上翻了一会儿,回到桌面就是那一屏(一块屏,两只手写)](35-one-screen-across-devices.md) — 人不认为自己有两块屏,他认为自己有一块屏、只是从两个地方看;而产品是一扇窗一个 cursor,所以手机上翻过的地方桌面完全不知道,而且**没有任何地方出错**——两块屏各自都对,只是解释它得先教会他一道缝。所以 cursor 从窗口搬进 appearance:`views/open` 写它、`GET /api/out/view` 送它、每扇窗都跟着,trail 变成**一条**列表(agent 的 show 和人的 move 都往里追加,记着是谁的手)。删掉 `POST /api/in/view`、`Attention`、以及围着「会过期所以要报年龄」的一整套——服务器自己拿着的事实对自己过不了期。认下的代价只有一个:人在桌面前戳一下手机,桌面会动(show 本来就会这么对他)。**已实测**(两扇窗跟随、刷新落在 cursor、到达发卡回头不重排、重启整行还在);实测当场改掉一处设计——到达要写快照,否则 agent 还没 show 过东西的 core 一重启,人走过的整行全没。band 里点真卡片仍待在有桌面会话的机器上复测。
- [36 · 在别的 app 里按一下侧键,它就看见我在看什么](36-show-your-screen-from-a-button.md) — 桌面 ⌘⌘ "come and see this" 的手机版:按操作按钮 → 系统截当前 app 的屏 → hi agent 收下并打开。关键约束是 **iOS 上没有任何 app 能给别的 app 拍照**,所以图必须由快捷指令的 Take Screenshot 拍、由 App Intent 接过来,且必须**先拍后开 app**,否则拍到的是 hi agent 自己。载体自己说这是什么(multipart 里一个 `note` 部分,和桌面在进程内填的是同一个字段),因为 Reaction 不开文件、只有那句话可判。Rust 半有集成测试,iOS 半编译通过但**没在真机上跑过**(模拟器没有操作按钮)。
