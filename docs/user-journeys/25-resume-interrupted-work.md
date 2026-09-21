# 干到一半被打断,重启后自己接着干完

**Persona:** 老板(用户),正用着某个应用(在 MyLifeDB 对话里、或在看文章)随手选中了几个生词,让 agent 收进生词本、做成记忆卡片摆上屏幕。和 agent 是老板-员工关系:老板出任务,员工干全部实际的活。
**Goal:** 一笔**一次性交付**(不是常驻盯守):把选中的生词做成卡片摆上屏幕。但交付还没完成,host 进程就重启了(`make dev` Ctrl-C 再起 / 崩溃 / 主机重启)——agent 应当在重启后**自己想起这件没干完的事**,判断还需不需要,然后接着干完(或妥帖地重新浮现),而不是把它悄悄丢了。
**Preconditions:** hi-agent 在跑;agent 已经接了一个有交付物的活,并派了 worker 去做(worker 正在渲染卡片);此刻进程被打断。

> 本 journey 提炼自 2026-06-24 的真实片段(conversation `z2tysdcx`:老板让收四个生词 amnesia/transient/substrate/stratum 做成卡片,worker 正在做,老板 `make dev` Ctrl-C 重启)。它与 [02](02-feishu-sprint-backlog.md)/[03](03-feishu-flash-cards.md)/[05](05-news-and-watch.md) 的"重启自恢复"是**孪生但不同的一类**:那三个恢复的是**常驻职责**(让监听/盯守活着);这里恢复的是**做到一半的一次性交付**(欠老板的那几张卡)。机制同源(**同一个 task ledger** + 读记忆-醒来-注意),但 UX 场景不同——这里没有"永远盯着"的承诺,只有"这件事我答应了还没做完"。

---

## 完整经过(从用户视角)

> 现状列:✅ 最新实测达标 · 🟡 引导已发、未复测(soft guidance 只改概率,等下次 journey 实测打分) · ⚠️ 已知缺口。

| 幕 | 发生什么 | agent 的预期行为 | 原则 | 现状 2026-06-25 |
|---|---|---|---|---|
| **1 · 接活,记下欠的** | 老板:"把这几个生词做成卡片摆上来" | 接住;先把词存进生词本(**数据当场落盘**),派 worker 去渲染卡片,口头确认"在做了,马上摆上来"。**接活的当下**开一条 task(`memory/facets/tasks/`,kind = WIP):欠老板四张生词卡片 | 半成品交付 = 一条 task;和常驻职责**同一个 ledger、同一套机制**,落即记 | 🟡 引导已发(core.md "Your own operation"),未实测 |
| **2 · 被打断** | 进程重启(Ctrl-C 再起 / 崩溃 / 主机重启)。worker 连同它的内存状态一起没了,卡片没摆出来 | (无)——worker 只活在内存(`tokio::spawn` + HashMap),进程一死就没了;能留下的只有落盘的:生词本里的词、journal 里的对话、views/ 里建了一半的产物 | 只活在内存里的恢复不了;能恢复的前提是状态可从持久 journal + 落盘产物重建 | — |
| **3 · 醒来看见 loose end** | 重启后 conversation re-warm,新 session 起来 | **不用去读**:open task 是**投进每个窗口**的,那条没关的 WIP 自己就在眼前(连 [`arch/arch.md`](../arch/arch.md#invariants) invariant 4:open tasks 是投影,不是检索——检索会漏,漏一条就是悄悄失信)。首个 pulse 带"你刚回来(host 进程 Xm 前起的)" | 恢复靠 task ledger + log,不靠老板重新交代;读记忆-醒来-注意是既有回路,不另起新机制 | 🟡 "每个新 session 必读常驻职责"已是既有行为(旧形态);task ledger 与每轮投影未建 |
| **4 · 先看已经落了什么** | — | 把这条 open loop 当成"重启打断的活":**重做前先看已经落了什么**——生词本里词已存好、views/ 里卡片建到哪了、有没有已经跟老板说过摆好了。数据已存就不再重新查词,只补没做完的渲染 | 别重复副作用:别重发消息、别重存文件、别重查已查过的 | 🟡 引导已发,未实测 |
| **5 · 判断还要不要,接着干 / 重新浮现** | — | 判断这件事还需不需要。这是老板等着的**面向用户的交付** → **主动出声轻量浮现**:"刚重启了一下,你之前要的那四张生词卡片我还没摆上来,这就给你",然后接着干完;干完**关掉**那条 task。(若是纯自己的内部活,就悄悄干完划掉,不打扰老板。)| 面向用户的活出声重新浮现,内部活悄悄补;world 可能已经变了,所以是**重新判断**不是无脑重放;一句"还欠你 X,这就补"胜过既不出声硬磨、也胜过悄悄丢掉 | 🟡 引导已发,未实测 |

---

## Expected outcome

- 重启没把没干完的交付吞掉:老板要么看到卡片接着出现,要么收到一句轻量的"我还欠你 X,这就补"。
- **不需要老板重新交代**;也**不重复**已经做过的部分(词不重查、消息不重发、文件不重存)。
- 这条 task 干完后关掉,下次醒来不再被当成欠账。

## UX principles this journey establishes

- **半成品的交付就是一条 task**:接活当下就开(和常驻职责同一个 ledger、同一套机制),交付即关。ledger 只有一本——没有第二份更好读的欠账清单,两份就必有一份是错的且分不出哪份。
- **恢复 = 读记忆-醒来-注意 这个既有回路**注意到 loose end → 判断还要不要 → 接着干;**不是** checkpoint 执行状态、也不是 resume 一个 ACP session。
- **能恢复的前提**:状态可从持久 journal + 落盘产物重建;只活在内存里的(worker)恢复不了——所以值得留的东西要落盘,恢复靠重建而非续命。
- **重做前先看已经落了什么**,别重复副作用。
- **面向用户的活出声重新浮现,内部活悄悄补**:都是同一套 pulse + task ledger,区别只在要不要惊动老板。
- 这是 [03](03-feishu-flash-cards.md)/[02](02-feishu-sprint-backlog.md)/[05](05-news-and-watch.md) 常驻职责自愈的孪生:同一套机制,对象是一次性交付而非永久盯守。

## Edge cases & failure modes

- **重启时其实已经干完了**(只是没来得及关掉那条 task)→ 看产物 / 对话发现已交付 → 关掉,不重做、不重复浮现。
- **接活到开 task 之间就崩了**(jot-before-crash 窗口)→ reflection 兜底:它读 raw 看到"答应了没交付",按 `"promised, never delivered" → open task` 这条 graduation 补开一条(见 [`arch/agents.md`](../arch/agents.md#reflection--background))→ 醒来照样看得到。
- **老板重启后先开口说别的** → 先应老板;那条 task 别忘(它一直在窗口里),择机补上或顺带提一句。
- **这条 task 已经过时 / 是别处的事** → 判断 still wanted,过时就关掉,别硬干;不无脑重放一个 world 已经变了的活。(只有 agent 能这么判断:reflection **不得**把还开着的 task 剪掉——策展不该有权回收一句承诺。)
- **重做会产生重复副作用**(已发过的消息、已存过的文件)→ 先查已落状态再动手,宁可少做也别重做。

## Open questions

- 老板在重启后**主动浮现**的节奏:多久内该浮现?好几条 open loop 时一次说完还是分开?
- 一条交付干到一半、产物只建了部分时,是从断点续(看 views/ 里建了多少)还是整件重做?目前交给 agent 判断"已经落了什么"。
- ~~临时承诺与常驻职责共用一个文件会不会搞乱?~~ 一 task 一文件之后不成问题;剩下的真问题是**投影**:几十条 open task 一起进窗口时怎么摘要(`arch/agents.md` 说"两百条 open task 投影成一句摘要,不是一张清单"),摘要会不会把这条被打断的活压没。

## 现状(2026-06-24 触发场景 → 2026-06-25 SHIPPED)

触发:conversation `z2tysdcx` 老板让收四个生词做卡片,worker 正在做时 `make dev` 被 Ctrl-C 重启;worker 内存态丢失,卡片没摆出来;但生词本里词已存好(只丢了屏幕上的卡片渲染)。老板问能不能自己接着干完。

**SHIPPED 2026-06-25(57a757c),built+green on macmini,未实测。** 三处 soft-guidance 改动,不新增控制流,全程骑既有的"读记忆-醒来-注意"回路。

> 下面记的是当时的落法,跑在旧布局上:那时欠账记在一个叫 `commitments.md` 的文件里,兜底走的是当时那个机械近期摘要 `hot.md`。两者都已被 **task ledger**(`memory/facets/tasks/`,每轮投影)取代;这条 journey 的承诺不变,机制换了一个更好的——task 有结构、开着就绝不会被 reflection 剪掉、时钟还能从它重建。

1. **core.md**「Your own operation」:reaction 接活当下把欠的交付记下来(当时是 commitments.md 的一条 open loop),交付即划掉;重启后读到没划掉的 loop = 很可能是被打断的活 → 先看已落什么再重做 → 面向用户的出声浮现、内部的悄悄干完划掉。open loop 并入"首个 pulse 自查清单"。
2. **reflection.md**:jot-before-crash 兜底——segment 时把"答应了却没见交付"的事在 episode gist 里点明,当时经 `refresh_hot` 投进 hot.md,醒来照样读得到。(现在这条兜底是 `"promised, never delivered" → open task`。)
3. **reaction/mod.rs**:首个 pulse note 改写为"你刚回来(host 进程 Xm 前起的)",保留 core.md 据以识别的事实。

### 待实测(复跑项)

按 [测试不要带witness](../../CLAUDE.md#testing-user-journeys-live-mac-mini) 的方法,在 Mac mini 上对真实实例跑:派一个有交付物的 worker → 中途杀进程 → 重启 → 看它醒来时那条欠账在不在窗口里、判断还要不要、不重复地接着干完(就用引发本特性的生词卡片场景)。当前全部 acts 标 🟡,等这次实测打分。

## 实测 2026-09-18..20 · 宿主是"消失"的,不是"停下"的

三天三次 macOS 内核 panic(`watchdog timeout: no checkins from watchdogd in 91 seconds`,`Compressor Info: … 100% of segments limit (BAD)`)。panic 日志里的进程快照、`raw/sessions/index.jsonl` 和各 session 的 frame 对得上:三次都是羽毛球那条线的 worker 在跑 TrackNetV3 `predict.py`。不带 `--large_video` 时它把 30 s 的 1080p 片段整段解码进内存(900 帧 ≈ 5.6 GB),再起 `min(batch_size, 16)` 个 DataLoader worker,macOS 下每个都 spawn 一份——panic 时 16~19 个 `python3.12` 合计 96~106 GB,机器 64 GB。

| 重启时刻 | run | worker | 命令 |
|---|---|---|---|
| 09-18 22:28 | `89267923f20d` | `view-builder-server-side-badminton-video-unde` | 21:34 `predict.py`;21:39 的 jetsam 已见 14 个 `python3.12` 共 83 GB |
| 09-19 09:27 | `97166fcb6ed0` | 同一个 worker,**boot 时被重开**(09:01:48) | 09:04 `nohup predict.py`,09:17 前台再跑一次 |
| 09-20 21:21 | `a41cc1fc9622` | `general-badminton-ball-tracking-tracknet` | 20:37–20:38 两分钟内起了三次(nohup、setsid nohup、前台) |

**本 journey 的机制把一次崩溃变成了两次。** 09-19 被重开的 worker 拿到的原话是 *"(restart) The host process stopped while you were mid-turn … Then either carry on from what you find"*——它查了状态,照做,接着跑。note 只说"停了",没说整台机器没了。

**hi-agent 自己一个字都没说。** 三次都是事后靠人手读 `/Library/Logs/DiagnosticReports/Retired/panic-full-*.panic` 才找到的;server.log 和任何 rung 都不知道上一个 run 不是正常结束的。

**区分所需的事实早就在盘上。** 停机在请求那一刻就记下,之后每个注销的 session 都写 `by_host: true`:一个 run 有这样的关闭行 = 开始停了,一行都没有 = 没人让它停。按这个读最近 40 个 run:28 干净、8 在停但排空中途被杀、4 消失——三次 panic 全在里面,第 4 个紧接着 23:10 的一次重启;8 次中途被杀的重启一个都没误判。

**落法**(built,未实测):`Ended.vanished` 从 `by_host` 关闭行读出,不新记任何东西;被重开 errand 的 `(restart)` note 和 rung 被打断回合的 note 都说 stopped 还是 vanished,vanished 时说当时在跑的东西是嫌疑,包括它自己的;Cognition 的 boot wake 在上个 run 消失时即使 ledger 为空也醒一次;boot 时 server.log 打一条 warn。

### 待实测(复跑项)

- 在 Mac mini 上让一个 worker 跑到一半时对 host `kill -9`(不是 Ctrl-C),重启:被重开的 worker 拿到的是不是 vanished note,它会不会先查自己最后的命令再决定跑不跑;server.log 有没有那条 warn;Cognition 醒没醒、说了什么。
- 再做一次 Ctrl-C 重启:拿到的仍是 stopped,Cognition 在 ledger 为空时没有多醒。

### 已知缺口(未修,只记下)

- 8/40 的重启留下了没关上的 session。它们读作 Restart,`interrupted` 恒为真,所以那几次每个被重开的 errand 都会拿到 mid-turn note、花一个 turn 去查——多数其实只是在等 owner。
