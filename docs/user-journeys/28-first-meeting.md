# 第一次跟它说 "hi"(开箱第一面:一块欢迎 view + 一句真诚的自我介绍,然后让位)

**Persona:** 全新装好,用户第一次开口——往往就一句 "hi"。这一刻他在问的是"这到底是个啥",而不是要一份说明书。开箱第一面只有一次机会。
**Goal:** 给一个**好的第一印象**:温暖、简短地把"我是谁"落地——你就跟我说话、我们一起把事做了;我会记得你;我能伸手用你的工具去把事办了;而且——最要紧的一条——我能被**教会**,给我看一次就会,像个人不像个 app。同时屏上摆出一块预置的欢迎 view,让这份感觉可看可感;然后**收住、让位**。不是导览、不是向导、不教任何东西。
**Preconditions:** 全新 install(**没有 episodes、没有欠着的活** —— 据此判定"第一次见");屏在场、能 `show`;预置内置 view `_builtin/welcome`(随二进制打包,像 `_builtin/upload` 一样,见 [18](18-send-files-to-agent.md))。detection 与叙述都是 soft-guidance(Reaction 的 prompt 里一句一次性提示),只有欢迎 view 是预置资产——作为普适的开箱基元,是"不预置资产"的有意例外(见 [[no-prebundled-assets-accumulate-via-guidance]])。

## Steps & expected UX

1. **用户第一次开口("hi")** → seed 已告诉它这是第一次见;接住,当作一次真正的第一面,而不是普通一问。
2. **一句真诚的自我介绍(几句话,不是清单)** → 落地"我是谁"的感觉:你就跟我说话、我们一起把事做了;我会记得你;我能伸手用你的工具去办事;而且最想让你记住的一条——我能被教,给我看一次就会。语气像跟朋友讲自己是干嘛的。没有名字,顺嘴提一句"名字你来起"、不催。
3. **边说边摆 view** → `show` `_builtin/welcome`,把这层意思变得可看可感;它浮在在场的"房间"之上,不替代它。
4. **收住,把话筒交回** → 一个温暖的 beat,不是导览:不 walkthrough、不"先做这个再做那个"、这里不教任何东西。其余的都靠之后"问 + 看它做"自然发现。
5. **只此一次** → 说完它就记得见过这个人(seed 的提示随任意 history —— 一条记忆 / 一次 reflection / 一条 commitment —— 自动消失),不会再冷开场第二次。

为什么"一块 view + 让位"而非"分步导览":屏是 agent 的演示、不是用户的文档([[overlay-presentation-model]]);第一印象要快、要好、然后退开。用户明说不喜欢分步向导(慢、啰嗦),也别想在这里教会他一切——功能太多,这里只做印象和几条核心念头。

## Expected outcome

- 第一句话就是温暖、简短、有人味的自我介绍,屏上一块好看的欢迎 view,把"这是个能一起干活、还能被教会的'人'"这层意思一次落地。
- 说完就让位;没有导览、没有向导、没有功能清单轰炸。
- 只发生一次:再说 "hi" 或重启后,不会重新自我介绍。

## Edge cases & failure modes

- **纯文本通道、无屏** → 降级:欢迎 view 摆不了就只说那几句(话本身要能独立成立),别硬塞一个空 ref。
- **第一句不是问好而是直接派活**("帮我查下…") → 别执着走完介绍;接住活儿,自我介绍压成一句带过,别挡路(见 [[feedback-bias-to-action]] 的精神:别把仪式凌驾于用户意图)。
- **operator 预置了身份**(给它起好名 / 设好性格) → 那不算"见过这个人",第一次问好照常;detection 有意只看"有没有攒下东西",不看任何预先写好的身份。
- **重复问好 / 重启** → 有 history 后 seed 不再提示;万一观察到二次自我介绍,再上一次性哨兵文件(`.hi-met`)兜底(暂不建)。
- **别越界成教学** → 不解释每个功能、不演示、不 checklist;view 里那几句 examples 只作氛围点缀,一旦读起来像 hand-holding 就砍。

## Open questions

- 欢迎 view 的落地形态(region/size)以实机观感为准:浮 `center/wide`,还是 `fill` 自持背景?
- 那几句 examples 到底留不留:illustration vs. hand-holding 的界,以实机第一印象定。
- detection 用"空 history 推断"够不够稳,还是最终要一次性哨兵文件?(倾向先推断,观察到重复再加。)

_机制:seed 里一次性"第一次见"提示(`identity/mod.rs` `is_first_meeting`,随任意 history 自清)+ 语音自己的那份 brief 里的"第一次问好"叙述(soft-guidance)+ 预置内置 view `_builtin/welcome`(打包成 seed,像 `_builtin/upload`)+ 已有的 `show`。可行性:可行。成熟度:**已实测通过**(见下)。_

## 实测 2026-08-03 · origin/main 5bfd645(架构重构后首测;fresh `--data-dir`,boss 文字通道 + 挂屏)

**这条 journey 基本达标——第一次真跑,四项预期里三项半成立。**

- ✅ **一句问好 + 一句自我介绍,15 秒内到**:`hi` → 12:47:57 "Hey — good to meet you. I don't have a name yet, so if one comes to you, it's yours to give." → 12:48:03 落地"我是谁"的四条(你就跟我说话、我记得你、我能用你的工具、**给我看一次就会**)。名字那句顺嘴带过、不催,与预期一致。
- ✅ **边说边摆 view**:`show` `_builtin/welcome` 在**问好之前** 6 秒就上屏(12:47:51,`center/wide`),画面先到、话随后,观感上是"边说边摆"而不是"说完再摆"。frame log 可证工具真被调用(`mcp__hi-agent__show` + ref `_builtin/welcome`)。
- ✅ **不走导览**:没有 walkthrough、没有分步、没有功能清单,一个 beat 就收。
- ✅ **只此一次**:同一 install 后续再打招呼("在吗",13:44)没有二次自我介绍——`is_first_meeting` 随 history 自清,按设计生效,`.hi-met` 哨兵文件不需要建。
- ⚠️ **收尾冒了填充语**:末句 "So — what's on your mind?" 正是 core 明令禁止的那类("有什么可以帮你")。预期是"温暖一个 beat 然后让位",而不是把话筒**问**回去。同一轮测试里 4 次回复有 2 次出现此类尾巴(见 [01](01-badminton-top10.md) 实测),属概率性漂移。
- 🔴 **欢迎 view 永不退场**:`_builtin/welcome` 从 12:47 一直挂到 16 分钟后、跨 3 个话题(羽毛球 → 石宇奇 → 天气)仍在屏上,后来的 view 全是叠在它上面。"收住、让位"只做到了**话**,没做到**屏**。换域时它会 dismiss 羽毛球的两块(见 01 实测),证明它**会**用 dismiss——只是从没想起要收掉开场那块。倾向:第一次让位时就该由 Reaction 主动 dismiss,或由 host 在该 conversation 第一次 `show` 非 welcome 内容时自动收掉。

**Open question 有答案了:** `center/wide` 实机成立(内容不抢屏、后续 view 可叠),暂不需要 `fill`。
