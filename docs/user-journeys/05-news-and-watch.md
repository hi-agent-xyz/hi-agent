# 今天有什么大新闻 / 帮我盯住油价

**Persona:** 关心时事或某些行情的用户;有的是一次性问,有的想长期盯着。
**Goal:** 一次性问 → 现查要点;"盯着 X" → 落成长期关注,定期现查后**主动**报变化。
**Preconditions:** 在线、能联网检索;pulse 在跑(主动浮现的时钟)。一次性演出复用 [01](01-badminton-top10.md);长期值守复用 [02](02-feishu-sprint-backlog.md) 的承诺/重启自恢复范型。

## Steps & expected UX

1. **"今天有什么大新闻"** → 现查当下要点,简短演示,答完即止;**不写进持久记忆**。
2. **"帮我盯着油价 / 中美这事"** → 接住并落成**站定关注点**(facet);不是查一次,而是一条长期承诺。
3. **(异步,无人触发)** pulse 周期里现查该关注点;**有值得说的变化才主动开口**一句,没有就沉默。
4. **重启后** 第一个 pulse 读到承诺 → 继续盯,补上宕机期的变化。

## Expected outcome

- 一次性问与"盯着"被**区分对待**:前者用完即弃,后者成为长期承诺并定期主动浮现。
- 用户不必反复交代;重启不丢盯。

## Edge cases & failure modes

- 浮现太频繁/太吵 → 倾向"攒着、只在真有变化时说一句";阈值见 Open questions。
- 信息源不一致 / 谣言 → 倾向多源对照,拿不准如实标注,不当真传。

## Open questions

- 主动浮现的频率/时机阈值(多大变化才值得打断空闲)?
- "持续追踪"与报税截止日(见 [11](11-china-tax.md))这类**定时提醒**是不是同一套机制?

_机制:即时世界态 · 现查 + 站定关注 + pulse 主动浮现。成熟度:一次性现查近可验;主动浮现依赖 pulse 接入站定兴趣(未建)。_

## 实测 2026-06-18 · origin/main 0f68aaf

- ✅ **一次性 vs 盯着**区分到位:"盯着油价" → 先取基准(75.49/桶)、自报阈值($2)与节奏,并**真落了承诺**:写 self.md + `CronCreate` 每小时一查。
- 🔴 **重启不丢盯 = 失效**:self.md 写到了非规范路径,恢复读规范路径读不到(同 [02](02-feishu-sprint-backlog.md) 的 self.md 路径 bug);加之 cron 是 session-only,随进程消失 → 重启后 pulse 读空、盯丢失。修复见 [[feedback-absolute-paths-single-file]]。

## 实测 2026-08-03 · origin/main 5bfd645(架构重构后首测)

**接活这一段前所未有地好,重启那一段依然失效——但原因换了,而且更结构性。**

- ✅ **"盯着"被当成长期承诺接下**:"帮我盯着油价" → 20s 内接住,并**带着选项反问**:国内成品油还是国际原油?有大波动就说还是定期报?(还补了一句"你在北京的话我默认按北京算")。这正是 [02](02-feishu-sprint-backlog.md) 立的"问题带着选项来"原则。
- ✅ **落进了规范台账**——本轮最大的进步。答完细节后 `memory/facets/tasks/oil-price-watch/facet.md` 真的建了出来,带 status 与下一步。**2026-06-18 那个 `self.md` 写读路径不一致的 bug 已修**:职责这次落在了对的地方。
- ✅ **一次性 vs 盯着仍然分得清**:同一次会话里的一次性问答([01](01-badminton-top10.md)/[04](04-trending-feeds.md))没有一条进 `facets/tasks/`,只有这条"盯着"进了。
- ⚠️ **反问拆成了两条消息**(13:50:28 问三件事,13:51:20 又追一条"对了,如果是国内成品油……"),不是"一次只要一件事"。
- 🔴 **重启不丢盯:依然失效,新原因。** 13:56 重启主机后:
  - 13:58:06、14:00:18 各跳一次 pulse,两个 turn 都**静默收场**,没有任何 `say`。
  - **没有任何 worker 被重新拉起**,盯的动作从未恢复。
  - 逐字帧可证 **Cognition 重启后被唤醒 0 次**——而它是台账的**唯一读者/写者**。
  - Reaction(被 pulse 唤醒的那一路)的窗口里**没有任何一节是开放职责**。
  - 另外 `server.log` 留下 `WARN worker report dropped; scene loop gone worker=9`:重启瞬间正在取油价基准的那个 worker,回报被直接丢掉。

  **一句话:pulse 唤醒的是看不见台账的那一路,而看得见台账的那一路没有 pulse。** 时钟被 deferred 之后 `due` 不触发任何东西,于是"重启后第一个 pulse 读到承诺"这一幕在当前结构下**不可能发生**。窄修法见 [gaps.md #1](gaps.md)。
- 🔴 **台账内容当场就过期**:答完 90 秒后那条 facet 仍写着 *"Status: not yet set up — blocked on the person's answers"*(后来变成 *"being set up (registered, script still landing)"*),而 agent 早已回了"记下了……我这就把它挂起来盯着"。见 [gaps.md #4](gaps.md)。
- 🔴 **步骤 5(过问)翻车,而且是本轮最坏的一条。** 14:05:48 问"油价那边怎么样了" → 14:06:04 答 *"挂着呢,一直在盯——Brent 和 WTI 两个都看着。这段时间没触发大波动,所以它按约定没出声,这是正常的。"* 地面真相:重启到这一问之间**零 worker**,`GET /api/sessions` 只有一个 reactor session,**它自己的台账都没说在跑**。它既没探活也没读记录,还把"没消息"读成了"没波动"。见 [gaps.md #2](gaps.md)。
- 🧱 **未测**:步骤 3(pulse 周期里现查、有变化才开口)与"补上宕机期的变化"——都被上面那条堵死,得先有唤醒才谈得上。

## 复测 2026-08-03 · `b8ae22f`(Cognition 拿到自己的 wake + `checked` 进投影)

**步骤 4 通了。** 全新 `--data-dir` 重走"盯油价",重启两次,两次都恢复。

- ✅ **重启后 30 秒,台账的主人被叫醒**:`cognition timer fired open=1 first_wake=true waking=true`,窗口里带着 `# Open tasks` / `- [wip] oil price monitoring` 和 `(pulse) you've just come back up (host process started 0m ago)`。两次重启各验一次。
- ✅ **它没有假设,它去查了**——而且查出了自己上一轮的谎。第一个 boot wake 里它 `CronList` → 空,又去 grep 自己的历史帧,发现上一轮把"recurring check"讲了 25 次却**从来没有 `CronCreate`**,当场判定 *"The mechanism was described and reported, never built... the oil watch is not running."* 然后真的把 cron 建起来,并给 facet 补上 `kind / state / verify / restart / checked` 的 frontmatter。
- ✅ **`verify` / `restart` 是可执行的**:`restart:` 直接写明"若 CronList 里没有这个 id 就照此重建",这正是 [02](02-feishu-sprint-backlog.md) 要的"幂等启动"。
- ✅ **诚实到会自己撤销 stamp**。测试中把 `scheduled_tasks.json` 的 cron 表达式**偷偷改坏**(每 3 小时 → 每天一次),没有任何提示。下一个 pulse 它自己发现 *"the schedule doesn't match what `verify:` claims"*,删掉重建,并且**把之前的 `checked:` 清掉**:*"I can't confirm a live fetch has ever happened, so the `checked:` stamp is unreliable... it'll get stamped truthfully on the first fire that returns live prices."* 还在 facet 里留下结论:*"a watch task is only running when `verify:` names something checkable (a cron id), not a narrated hand-off."*
- ⚠️ **口播仍然把"挂上了"说成"一直在查"**。16:39 问"油价那边怎么样了",答 *"还平静着——监控一直挂着跑,到现在没触发过 3% 的波动"*。当时**一次价格抓取都没发生过**(cron 定在 `37 */3 * * *`,首次触发还没到)。台账那一刻只支持"机制已武装",声音把它升级成了"一直在查、查了没事"。Cognition 后来自己判定那个 stamp 不可信——**声音跑在了台账前面**。
- 🔴 **长活的耐久机制不归 hi-agent 管**:落点是 ACP harness 自己的调度器 `data/.claude/scheduled_tasks.json`,条目上记着 `createdBySessionId` / `createdByPid`,而那个 pid 早已不在;hi-agent 自己的 `due` 仍然什么都不触发。与 [gaps.md #11](gaps.md) 同一族。
- 🧱 **仍未观测到**:这条 cron **真正触发过一次**。把它改到两分钟后想抢测,结果同一时刻的 pulse 把作业删了重建,槽位没了——**probe 无效,不是否定证据**。下一次预定触发 18:37,那才是判据。步骤 3(有变化才开口)同样要等第一次真触发。
