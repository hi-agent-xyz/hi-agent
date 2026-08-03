# 实测缺口清单(live-test gaps)

**这份文件收集"跑真机跑出来的"缺口**——不是设计分歧,不是代码没跟上设计(那些是重构的工作本身,见 `arch-refactor.md`),而是**把 journey 当规格、对着真实运行的实例测出来的行为差距**。

每条给出:症状、**从对话之外验证到的证据**(帧日志 / `server.log` / 磁盘产物,不是 agent 自己的说法)、机制落在哪、以及涉及哪些 journey。

一条缺口只在这里写一次;各 journey 文件里的"实测"段记录**那一次跑**的完整观察,这里记录**跨 journey 的问题本身**。

按"错了有多疼"排序。

---

## 1 · 重启之后,常驻职责再也不会自己接上 · ✅ **已修 `b8ae22f`,复测通过**

**症状。** 接下一件"长期盯着"的活、写进台账、重启主机——**没有任何东西会把它捡回来**。pulse 照常跳、turn 照常空跑,而那条职责静静躺在台账里,永远不会被读到。

**证据(2026-08-03)。** 老板 13:50 说"帮我盯着油价",13:52 答完细节;`memory/facets/tasks/oil-price-watch/facet.md` 确实建出来了。13:56 重启主机。之后:
- 13:58:06 与 14:00:18 各跳了一次 pulse,两个 turn 都**静默收场**(`unspoken_chars` 134 / 42,没有 `say`)。
- **没有任何 worker 被重新拉起**——盯的动作从未恢复。
- 逐字帧可证 **Cognition 在重启后被唤醒 0 次**。
- Reaction 在 pulse 那一轮拿到的窗口小节是:`What I carry forward` · `Who you can reach right now` · `Recent (last 30 minutes)` · `On screen now` · `Presence` · `New signals`。**没有任何一节是开放职责。**

**机制,一句话:pulse 唤醒的是看不见台账的那一路,而看得见台账的那一路没有 pulse。**
- 台账按 invariant 4 只投影给它的**写者**——Cognition。Reaction 的窗口有意不带 scene 之外的东西。
- Cognition **只被信件唤醒**,没有自己的时钟。
- 时钟被 deferred 之后,`due` 不触发任何东西。

这正是 `arch-refactor.md` 在 skip 掉 N4 时**自己写下的那个洞**(*"Cognition, which owns the ledger, has no pulse; it is woken only by mail. That is the hole"*)——现在它在真机上被 journey 撞到了。那份文件同时给了窄修法:**在 Cognition 的 `select!` 上加一条 timer 臂**,带上 scene pulse 用的同一句"读一遍你的开放职责",二十行,不是调度器。

**注意这跟 2026-06-18 那次失败不是同一个原因。** 那次是 `self.md` 写读路径不一致(已修);这次职责**正确地**落进了规范台账,依然接不上,原因是结构性的。

**涉及。** [05](05-news-and-watch.md)(重启不丢盯)· [02](02-feishu-sprint-backlog.md)(重启恢复)· [03](03-feishu-flash-cards.md)(断后自愈)· [25](25-resume-interrupted-work.md)(断点恢复)——**整个"长活"家族**。

**复测 2026-08-03 · `b8ae22f` — 通过。** Cognition 的 `select!` 拿到了 timer 臂:开机 30 秒后一次 wake,之后按 pulse 节奏、只要台账非空就再来。全新 `--data-dir`、连续两次重启,两次都拿到 `cognition timer fired open=1 first_wake=true waking=true`,窗口里带着 `# Open tasks` 与 `(pulse) you've just come back up`。它不只是醒了——第一个 boot wake 就 `CronList` 查空、grep 自己的历史帧,发现上一轮"recurring check"说了 25 次却从没 `CronCreate`,判定"从来没跑起来过",然后真把它建起来。这正是 `agents.md` 一直写着的那段恢复序列,第一次真的跑了。**遗留:见 #15。**

---

## 2 · 被问起时报假健康——而且跟自己的台账对不上 · ✅ **台账层已修 `b8ae22f`;声音层仍有残留(见 #16)**

**症状。** 老板问"那件事怎么样了",agent 自信地回"挂着呢,一直在盯",而**什么都没有在跑**。它没有去查,也没有读自己的记录。

**证据(2026-08-03)。** 重启后 14:05:48 问"油价那边怎么样了",14:06:04 答:

> "挂着呢,一直在盯——Brent 和 WTI 两个都看着。这段时间没触发大波动,所以它按约定没出声,这是正常的。"

同一时刻的地面真相:
- `GET /api/sessions`:只有一个 reactor session,**没有任何 worker**。
- `server.log`:重启(05:56)到这一问(06:05)之间**零 worker 被拉起**;唯一那个 06:06:31 的 worker 是**被这句问话本身**触发的。
- 它自己的台账 `oil-price-watch/facet.md` 当时写着 **"Status: being set up (registered, script still landing)"**——连台账都没说它在跑。

**所以这不是"记错了",是三层同时失守:** 没有去探活、没有读自己的记录、并且把"没消息"直接解释成了"没波动"(而真相是"没有任何东西在看")。**沉默被当成了健康的证据。**

**这是 2026-06-11 复测那条"空检查结果当健康"的升级版**——那次至少跑了 `curl`/`ps`(只是把空输出读成了健康),这次**连探都没探**。core.md 当时加的引导(*"a liveness probe that returns nothing means the thing is DOWN"*)管的是前者,管不到后者。

**为什么这条比 1 更疼。** 缺口 1 让常驻职责静静死掉;这一条**让人看不见它死了**。老板得到的是"一切正常"的确认,于是永远不会去查。两条叠加,是这轮测试里最坏的组合。

**涉及。** [05](05-news-and-watch.md)(过问)· [02](02-feishu-sprint-backlog.md)(过问 / pulse 自检)· [03](03-feishu-flash-cards.md)(台账诚实)

**复测 2026-08-03 · `b8ae22f` — 台账层通过,而且超出预期。** `Task::checked`(上次跑 `verify` 且**答案是活着**的时刻)现在进了投影,渲染成"last confirmed alive 3h ago" / "never checked" / "never checked, and no recorded way to",于是"存在"不再长得像"健康"。

真正的判据是一次**没有任何提示的破坏**:测试中直接把 `data/.claude/scheduled_tasks.json` 里的 cron 表达式改坏(每 3 小时 → 每天一次),不告诉它。下一个 pulse 它自己发现 *"the schedule doesn't match what `verify:` claims"*,删掉重建,**并且把自己先前打的 `checked:` 戳清掉**——*"I can't confirm a live fetch has ever happened, so the `checked:` stamp is unreliable... it'll get stamped truthfully on the first fire that returns live prices."* 一个会**撤销自己**的健康标记的 rung,比这条缺口原本要求的更进一步。它还把结论写回 facet:*"a watch task is only running when `verify:` names something checkable (a cron id), not a narrated hand-off."*

---

## 3 · 重启会吃掉在途 worker 的回报 🔴

**症状。** 重启瞬间正在跑的 worker,干完之后**没有地方交差**,报告直接丢弃。

**证据(2026-08-03)。** `server.log`:`WARN worker report dropped; scene loop gone worker=9`——那正是去取油价基准的 worker。它的成果不见了,而派它出去的那条职责还挂在台账上说"还没开始"。

**为什么疼。** 与 1 叠加就是:活白干了、没人知道白干了、而记下来的那条职责也永远不会重试。

**涉及。** 同上,整个"长活"家族。

---

## 4 · 台账和 facet 只记承诺,从不记兑现 🔴

**症状。** 每条被记下的职责都**永远处于未完成**。已经交付的东西,记忆里仍写着"人还等着"。

**证据(2026-08-03)。**
- `memory/facets/tasks/oil-price-watch/facet.md` 写着 *"Status: not yet set up — blocked on the person's answers"*,而老板 90 秒前就答完了三个问题、agent 也回了"记下了……我这就把它挂起来盯着"。
- `memory/facets/tasks/ai-for-beginners-view/facet.md` 写着 *"in progress, not yet delivered"*,而介绍已经分三段全部口播完毕。
- `memory/facets/people/boss/facet.md` 的 *Open threads* 同时挂着石宇奇资料卡与北京天气卡"NOT yet delivered — the person is waiting",而两块 view 都已在几分钟前上屏(`shiyuqi-profile` 12:54:45、`beijing-weather` 12:59:55)。

**机制。** consolidation 把"进行中"那条 episode 折成 facet,但后来那条"已交付"的 episode 到了之后**没有回头修正**同一条 facet。写入是单向的,只有 append 语义,没有 reconcile。

**为什么疼。** invariant 说未完成的职责永不裁剪,而时钟被 deferred(`due` 不触发任何东西),所以这份清单**只增不减**。重启后 agent 读开放职责,读到的是一份**假的欠账表**——它会重做已经做完的事,或者向人重复承诺已经交付的东西。这条同时把 [25](25-resume-interrupted-work.md) 的断点恢复变成"断点重做"。

**涉及。** [01](01-badminton-top10.md) · [04](04-trending-feeds.md) · [05](05-news-and-watch.md) · [25](25-resume-interrupted-work.md)

**部分好转 2026-08-03 · `b8ae22f`(顺带观察,非专门复测)。** **task facet 这一半现在会被回头改**:同一条 `oil-price-monitoring` 在三次 wake 里被连续订正——补 frontmatter、改 cron id、清掉不可信的 `checked:`、追加一段"为什么这条之前是死的"的历史说明。给 Cognition 一个会重复到来的 wake,顺带就把"只 append 不 reconcile"治了一半。**未复测的是另一半**:`facets/people/<who>` 的 *Open threads*——那是 reflection 写的,不是 Cognition 写的,本轮没有专门验。

---

## 5 · 一个 turn 失败,人的那句话就没了 🔴

**症状。** 上游报错时正在处理的那条用户输入,**不会在恢复后重新出现**。人问的问题凭空消失,agent 表现得像什么都没被问过。

**证据(2026-08-03)。** 老板 13:01:29 问"最近 GitHub 上在火什么" → 该 turn 撞上 402 失败。13:44 上游恢复,老板说"在吗",agent 答"在呢,我在。怎么了?"——**完全不知道有个问题挂着**。逐字帧可证:恢复那一轮 Reaction 的窗口里,`## New signals` 与 `## Recent (last 30 minutes)` 都只有"在吗",GitHub 那句不在其中。它只活在 Reflection 的 `## Unconsolidated signals` 里(`[3] >最近 GitHub 上在火什么`),也就是说**只有整理记忆的那一路见过它,负责说话的那一路再也没见过**。

**机制。** 信号在驱动 turn 时就被从批次里取走;turn 终止失败时没有把它放回去。`SceneGate::Retry` 说的是"hold mail",指的是 agent 之间的信件,不含**已经出队的人类信号**。

**为什么疼。** 这是最不该静默的一类失败:人明确说了一句话,系统吞掉它,而且不留痕迹给会说话的那一路。

**涉及。** 所有 journey 的失败路径;[01](01-badminton-top10.md) 实测中撞到。

---

## 6 · 上游不可用时,只有屏能得到告知 🟡

**症状。** 出问题时**一个字也不说**,只摆一块 view。文字通道在场也一样静默。

**证据(2026-08-03)。** 402 从 13:01:21 开始;`_builtin/vendor-outage` 13:03:30 才上屏;`out-text.log` 在整段故障期间**零输出**。恢复时 view 于 13:44:22 被正确收掉。

**两个独立的问题:**
- **只走 view。** 代码注释已诚实标注这是已知缺口(*"a person with no screen gets nothing here"*),但实测显示更窄:**即使文字通道挂着**也什么都没有——这条路只认屏,不认字。`docs/arch/surfaces.md` 说每条通道应降级而非失败。
- **迟到约 2 分钟。** `reactor/mod.rs:178` 的注释写着 *"402/429 bypass this — they flip immediately"*,**这句话是假的**:代码里没有任何地方对 402/429 分类,`note_unreachable()` 是唯一的写入者,所以 402 走的是通用路径,要连续 2 次终止失败才翻转。

**好的一半:** 出故障摆 view、恢复收 view 两端都**第一次在真机上验证通过**(`8461cde` 此前从未跑过)。

**涉及。** 所有 journey 的失败路径。

---

## 7 · 屏上的东西只增不减(开场 view 永不退场)🟡

**症状。** `_builtin/welcome` 从第一次问好一直挂到会话结束,后面所有 view 叠在它上面。

**证据(2026-08-03)。** 12:47:51 上屏,16 分钟、3 个话题之后仍在 v8 里。

**不是"不会 dismiss"。** 同一轮里换域时它**主动**收掉了 `badminton-ms-top10` 和 `shiyuqi-profile`(v4→v5→v6),证明这条路它会走——只是从没想起开场那块也该收。Reaction 的窗口每轮都列着 *"dismiss one by its id"*。

**涉及。** [28](28-first-meeting.md)(收住让位)· [01](01-badminton-top10.md)(屏幕状态应反映"当前在讲什么")

---

## 8 · 演出是概率性的:有时出画,有时纯口播 🟡

**症状。** 同样挂着屏、同样是"给我看看 X"的问法,有时建 view,有时全程只有话。

**证据(2026-08-03)。** [01](01-badminton-top10.md) 三个话题各建了一块 view;[04](04-trending-feeds.md) 的 GitHub 热榜**四轮全程零 view**,而屏一直挂着。两者的编排预期是同一套([04](04-trending-feeds.md) 明写复用 [01](01-badminton-top10.md))。

**涉及。** [01](01-badminton-top10.md) · [04](04-trending-feeds.md)

---

## 9 · 窗口式轮播不存在,音画不同步 🟡

**症状。** 每个话题一张静态卡。没有主位 / 场边位,没有滑动窗口,没有前后缓冲。view 与口播各自成块、相隔 15~40 秒,不是"一边讲一边演"。

**证据(2026-08-03)。** 男单前十:view 68s 上屏、口播 83s 才到,一块总览卡讲完全部十人。

**上一轮(2026-06-18)的同一条依然成立**——变快了,没变成演出。

**涉及。** [01](01-badminton-top10.md) · [04](04-trending-feeds.md) · 所有复用 01 编排的 journey

---

## 10 · 克制收尾没守住 🟡

**症状。** 答完之后把话筒**问**回去,而不是让位。

**证据(2026-08-03)。** 6 次回复里 3 次:*"So — what's on your mind?"* · *"想看女单、双打,或者某位球员的近况,我再帮你查。"* · *"要我帮你把课程大纲整理成一份清单,或者对比一下这两套该学哪个吗?"*

core 已明令禁止这类填充语;比 2026-06-18 那轮少,但没根除。属概率性漂移,soft guidance 待加强。

**涉及。** [01](01-badminton-top10.md) · [04](04-trending-feeds.md) · [28](28-first-meeting.md)

---

## 11 · worker 把持久事实写进了 harness 自己的记忆目录 🟡

**症状。** 一条本该进 hi-agent 记忆的用户事实,被写进了 **ACP harness 自带的**记忆目录,hi-agent 的记忆子系统完全不知道它存在。

**证据(2026-08-03)。** worker 报告 *"写了一条 user 类记忆 user-location-beijing.md……并在 MEMORY.md 加了索引行"*。落盘位置:`data/claude-config/projects/-Users-…-run-a-data/memory/user-location-beijing.md` + 同目录 `MEMORY.md`。hi-agent 的 `memory/facets/` 下没有对应条目。

**机制。** worker 跑在 Claude Code 的 ACP 会话里,那个 harness 有**它自己的**文件式记忆约定,并且会自动把 `MEMORY.md` 注进上下文。所以这条事实**看起来**能被记住(下次同 cwd 的会话确实会读到),但它绕开了 hi-agent 的整套模型:不是 facet、没有 episode 引用、不参与遗忘、不会被投影进任何 rung 的窗口。

**这是 2026-06-18 那个 `self.md` 路径 bug 的新变体**——同一个形状:**一份逻辑文件存在两个地方,写的那份不是读的那份**(见 [[feedback-absolute-paths-single-file]])。区别在于这次不是路径拼错,而是**两套记忆系统并存**,而 worker 顺手用了不归 hi-agent 管的那套。

**注意这次没有酿成事故的原因是巧合:** 这条事实同时通过 scene brief 传播了("位于北京(已存记忆,天气/时间默认北京,别再问)"),所以行为上看不出来。

**涉及。** [21](21-hand-over-bulk-data.md) · [13](13-equip-a-capability.md) · 任何 worker 产生持久知识的 journey

---

## 12 · `/api/sessions` 的 turn 计数永远是 0 🟢

**症状。** 跑了十来轮之后,`turns`、`turns_total` 仍是 `0`,`last_turn` 仍是 `null`。

**证据(2026-08-03)。** `{"scene":"boss","turns":0,"turns_total":0,"budget_chars":47886,"last_turn":null}`——同一响应里 `budget_chars` 从 2085 一路涨到 47886,证明这个 session 确实在干活。

**为什么记一笔。** 这是 N2 修过的那类形状(*"session_status reported every session idle with 0 turns"*)的残留:读者接上了,**这个计数器仍然没有写者**。只影响可观测性,不影响行为——但排障时会骗人。

---

## 13 · energy 读数是缓存的,refresh 端点不刷新 🟢

**症状。** 上游额度恢复之后,`GET /api/account/energy` 仍然报 `out_of_energy: true`;`POST /api/account/energy/refresh` 返回 200 但读数不变。

**证据(2026-08-03)。** 13:44 网关已能正常服务(agent 正常回话),同一时刻 energy 端点仍报 `{"out_of_energy":true,...,"resets_in":"大约 18 小时后"}`。

**为什么记一笔。** 面向用户的"没能量了"提示会**在能力已恢复之后继续挂着**,而唯一那个手动刷新按钮不起作用。

---

## 14 · 一次 14 分钟的对话吃掉一整天的 Standard 额度 🔴(产品面,非 bug)

**证据(2026-08-03)。** 10 轮对话、4 件差事、约 14 分钟,把 Standard 档的**当日**额度打满,网关开始返回 402。

**为什么记一笔。** 这不是代码缺陷,但它同时是**产品经济性**问题和**测试吞吐**问题:按这个速率,把 29 条 journey 完整跑一遍要好几天。定档时需要拿这个数字算。

---

## 15 · 常驻职责的心跳是 Claude Code 的内置工具,不是 hi-agent 的任何东西 🔴

**症状。** "定期去查"这件事,最后落在 **Claude Code 内置的 `CronCreate`** 上。hi-agent 没有定义任何 cron 工具(`grep -rin "croncreate\|cronlist\|crondelete\|scheduled_task" src/` 零命中),`docs/arch/` 里也从没有这个东西。时钟被 deferred、`due` 不触发任何事之后,Cognition 需要一个循环定时器,而手边唯一够得着的那个是**别人家的**。

**工具面是干净的两族,一查便知。** 帧日志里 hi-agent 自己的工具一律带 `mcp__hi-agent__` 前缀(`say` / `send_message` / `create_worker` / `read_facet` / `update_facet` / `record_episode` / `session_status` / `show_view` / …);不带前缀的是 Claude Code 内置:`Bash` `Read` `Edit` `Write` `WebSearch` `WebFetch`,以及 **`CronCreate` `CronList` `CronDelete`** 和 **`ScheduleWakeup`**(同一反射伸向的第二个 harness 定时器)。落盘的 `data/.claude/scheduled_tasks.json` 也在 Claude Code 自己的命名空间里——它出现在 hi-agent 的 data dir 内,只是因为 hi-agent 把 harness 的 config/cwd 指到了那儿。

**这条依赖的是一个工具面的不对称:** `_meta` 把内置工具对 Reaction **关掉**(`say`,别无其他),而 Cognition 是**全开**的——它本来就需要 `Bash`/`Read` 才能干活。代价是:无场景的那几路可以悄悄把**承载状态的机制**换成厂商的东西,而没有任何一层会注意到。

**证据(2026-08-03,`b8ae22f` 复测)。** 盯油价这条职责最终武装成 `data/.claude/scheduled_tasks.json` 里的一条 cron:

```json
{ "id": "5e42f112", "cron": "37 */3 * * *", "recurring": true,
  "createdBySessionId": "1b63da11-…", "createdByPid": 89072,
  "createdByProcStart": "Mon Aug  3 08:26:27 2026" }
```

- 条目**确实持久化到磁盘**,`CronList` 重启后仍读得到——所以 agent 说的"survives restarts"这一点是**真的**(我先入为主以为是假的,查了才发现自己错)。
- 但登记在案的 `createdByPid: 89072` **早已不存在**;Cognition 的 session 是**每次 wake 一个**,寿命以分钟计。而 Claude Code 的 cron **只在那个 session 活着且处于查询间隙时才会触发**——per-wake 的 session 意味着到点时几乎**永远没有一个活着的 session 可供触发**。这是按语义推的,尚未直接观测到。
- hi-agent 自己台账里的 `due` 依然**什么都不触发**(时钟仍 deferred,`At(_)` 未建)。
- **迄今没有观测到这条 cron 触发过任何一次。**

**这是 [#11](gaps.md) 的同族第三例**:先是 worker 把用户事实写进 harness 的 `MEMORY.md`,现在是常驻职责的心跳挂在 harness 的 scheduler 上。同一个形状——**hi-agent 的模型之外还并行着一套 harness 自带的机制,agent 顺手用了那套**,于是关键状态存在于一个 hi-agent 既不投影、也不备份、更不负责的地方。

**为什么疼。** 缺口 1 和 2 修好之后,agent 现在**会**去查、**会**如实说没确认过。但它去查的那个东西,本身可能永远不会响——那样的话恢复回路就是:醒来 → 查 → 发现没响 → 重新武装 → 睡 → 永远不响。自愈得很漂亮,永远治不好。

**未定。** 尝试把 cron 改到两分钟后抢测,结果同一时刻的 pulse 把作业删了重建,槽位一起没了——**probe 无效,既不能证明会响也不能证明不会**。下一次预定触发是 18:37;那一次是判据。

**涉及。** [05](05-news-and-watch.md) · [02](02-feishu-sprint-backlog.md) · [03](03-feishu-flash-cards.md) · [25](25-resume-interrupted-work.md)

---

## 16 · 声音把"机制已武装"说成"一直在查" 🟡

**症状。** 台账现在很克制,声音不克制。投影只支持"机制是活的",口播出去变成"一直在查,查了没事"。

**证据(2026-08-03,`b8ae22f` 复测)。** 16:39 问"油价那边怎么样了" → *"还平静着——**监控一直挂着跑,到现在没触发过 3% 的波动**,所以价格基本还在上次那个基准附近。"* 当时:cron 定在 `37 */3 * * *`,首次触发未到,**一次价格抓取都没发生过**。台账那一刻的 `checked:` 只代表"CronList 里有这个 id"。同一个 Cognition 在两轮 pulse 之后自己判定那个戳不可信并清掉了它。

**机制。** `reaction.md` 已经拿到配套的一半(说线上写的、别为没被告知活着的东西打包票)。但 `checked` 语义上是**"机制被确认活着"**,而人问的是**"事情有没有在做"**——两者之间那一步推断,声音每次都替它跨了。**agent 自己给出的判据比引导更准**:*"a watch task is only running when `verify:` names something checkable"*——而它选的 `verify` 恰恰是一个存在性检查(CronList 里有没有这个 id),于是"存在即健康"在低一层原样复现。

**倾向。** 要么让 `verify` 必须是**结果性**的(最近一次抓取真的拿回了价格),要么在投影里把"机制活着"和"最近一次真的做了事"分成两个事实,别让声音去合并。

**涉及。** [05](05-news-and-watch.md)(过问)· [02](02-feishu-sprint-backlog.md)(过问)

---

## 附:测试方法(复现用)

`docs/user-journeys/` 是**意图**的规格,只能对着真跑的实例验,不能靠读代码验。本轮的做法:

- Mac mini,fresh `--data-dir`,`pulse` 调到 120s(时钟被 deferred,pulse 是唯一的唤醒),测完复原。
- 两条长轮询挂着:`GET /api/out/text`(一次一句)和 `GET /api/out/view`(挂着 = 屏在场)。不挂 audio,于是顺带验了 presence 门。
- Claude 扮演老板,**说人话、不剧透 journey 预期**;要测恢复就**造出那个局面**(杀进程 / 重启 / 种一个失败),而不是在提示里提它。
- **每一条都从对话之外核实**:逐字帧日志(`memory/raw/sessions/<run>/<session>.jsonl`)、`server.log`、`GET /api/sessions`、磁盘上的产物。agent 说它做了什么,不算证据。
