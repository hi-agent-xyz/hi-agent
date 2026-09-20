# 首页这几摊事应该放在一起

**Persona:** 手上同时有六到十几件事的用户,打开首页想一眼看清楚"我在忙什么";
他心里的分法不一定是任何字段能推出来的 —— 两个 KT 工单和一张粤语解说表是一摊事,
agent 自己身上的几个毛病是另一摊。
**Goal:** 首页按他心里的样子分组,而且改法就是说一句话;说过一次之后,新开的任务
也照这个分法落位,不用再说第二遍。
**Preconditions:** `factory/home` 正常;任务账本里有在办的任务;Cognition 在跑。

## Steps & expected UX

### 谁负责哪一段

| | 写什么 | 为什么是它 |
|---|---|---|
| Reaction | 什么都不写 | 原话转出去,和它做不了的任何事一样 |
| Cognition | `home/grouping.md`(依据) | 它是**听见**的那个;这句话不能因为后面的 worker 起不来就丢了 |
| `task-manager` | `home/groups.json`(结果) | 一次看全部在办行的判断,而且"同时只有一个"是它本来就有的纪律 |

### Case A · 他说一句,分组就变

1. **用户说**:"粤语解说那个和 KT8 的放一起吧。"
2. **Reaction 不自己动**,把原话转给 Cognition —— 这不是它能改的偏好,也不是要劝的事。
3. **Cognition 在同一轮里把原话按日期写进 `home/grouping.md`**(散文,没有任何代码解析它),
   然后起一个 `task-manager`(已经有在跑的就 `hi_send_message` 给它),brief 里带上原话。
   顺序不能反:文件是让这句话对 *下个月新建的任务* 仍然生效的东西。
4. **task-manager 读全部在办任务 + 整份 `grouping.md`,调 `hi_set_home_groups` 整份重写。**
5. 首页在下一次刷新时就是新的样子。**从说完到变,是一个 worker 起身的时间**,不是几秒 ——
   人不用盯着屏幕等,Reaction 先答会变成什么样。
6. task-manager 在报告里说它改了什么、依据是什么,和它说自己关了哪些行一样。

### Case B · 新任务在下一次整理时归位

1. 用户交代一件新的 KT 工单相关的事,Cognition 建任务、写记录。
2. **这一刻不分组。** 归位不是开一行的一部分,为一次归位专门起一个 worker 也不值。新卡先
   挂在核心旁边 —— 这也是诚实的画面:它刚进来,还没人整理过。
3. 下一次 `task-manager` 跑的时候(用户又说了什么、或者账本需要判断),它读 `grouping.md`
   里那句"KT8 那摊"的旧话,把新行放进 KTV 组。**用户仍然没说第二遍**,只是没有在那一秒完成。
4. **放不进任何一组的任务就不放**。挂在核心旁边是正常状态,不是缺陷;为了填满一个格子
   发明一个组,是一个没人能背书的主张。

### Case C · 他不满意

1. 用户说"这个不该在那组"、"把 KTV 放前面"、"别分了"。
2. 同样走 Reaction → Cognition(落原话)→ task-manager(改排布);"别分了"就是 `groups: []`。
3. 分组的先后就是文件里的先后,组内任务的先后也是 —— 没有任何"重要程度"的推断。

### Case D · 新组先戴默认图标,画好了自己换上

1. 一次排布里出现了一个新的组名(比如"学习类")。它先戴默认图标 —— 一叠卡片,和其它
   还没画的组一样。
2. `hi_set_home_groups` 的回执每次都带 `the picture every icon is an edit of: ⟨ref: drive/home/group-icon.png⟩`,
   另起一行点名还戴着默认图标的组:`default icon still: 学习类`。
3. **同一个 task-manager 接着画**:挑一个代表"这一组对他来说是什么"的东西(不是把组名
   画出来),用 `hi_image_to_image` 从那张默认图改,只换物体,背景、配色、平面画法、构图
   全都不动,不带字。看一眼画出来的东西,再把排布整份发一次,这组带上 `icon`。
4. 首页下一次刷新,那个组换成自己的图标。**几周后新出现的组画出来,和今天的是一套**,因为
   它们都是同一张图改出来的。
5. 之后的整理不重画:不带 `icon` 的重写保留这个组名原来的图标。改了名的组是新组名,
   除非 manager 把旧图标带过去。
6. **但"重画"是可以开口要的**:那张所有图标都从它改出来的底图会变(换了风格、换了尺寸),
   一换,已经画过的和以后画的就是两套。回执每次都给底图的 ref,所以说一句"把图标都重画一下"
   就够 —— task-manager 拿那个 ref 把每个组重画一遍,再整份发回去。**没人说就不重画**:
   整理不是重画,忘了带 `icon` 不该让每个组赔上一次生成。

## What must be true

- **代码一条都不推断。** 不按 `systems`、不按标题相似、不按 episode 重合。被删掉的那个
  主题层就是推出来的:`systems` 说的是任务碰了什么,于是照片走飞书的生日 PPT 被画进了
  "feishu"。证据归写分组的人用,不归代码用。
- **分组只有首页读。** 任务记录里没有任何字段说它在哪一组,删掉 `data/home/` 只损失排布。
- **坏掉就不分组。** 文件缺失、JSON 坏了、成员全都不存在 —— 一律退回"每个任务挂核心",
  日志里有原因,用户屏幕上没有错误卡。
- **写错当场就知道。** 拼错的 subject 被丢掉并在工具回执里点名,还会告诉调用的那个
  task-manager 哪些在办任务没进任何组。
- **一只手写排布。** `hi_set_home_groups` 只有 `task-manager` 能调,别的 worker、Cognition、
  Reaction 一律在 dispatch 被挡回去。整份覆盖写,两只手不是合并而是互相抹掉。
- **听见的人负责把话留下。** 原话由 Cognition 当轮写进 `grouping.md`,不依赖 worker 起得来。
- **没有任何定时器。** 排布只在有人说了什么、或者账本本来就需要一次判断时才会变。
- **图标是写分组的那只手画的,代码不从组名猜。** 每一个都是同一张默认图改出来的,
  `hi_text_to_image` 从零画的不算。

## 实测 2026-09-16(本机,scratch data dir,release 构建)

### 完整闭环走通了一遍,是真人一句话进去

对一个从没分过组的实例(没有 `grouping.md`,没有 `groups.json`)发一句
`粤语解说表那个和 KT8-046 是一摊事,首页上放一起吧,就叫 KTV`,然后什么都不做:

1. Reaction 答"收到",没有自己动手。
2. Cognition 把原话按日期写进 `home/grouping.md`,还在里面记了它的理解(哪句话对应哪个
   subject),并注明"若记录里另有同名,以原话里的名字为准"。
3. Cognition 起了一个 `task-manager`,名字就是这件差事。
4. 那个 manager 读了 `grouping.md` 和全部在办任务,调 `hi_set_home_groups`,回执
   `home groups: KTV (2)` + `open and in no group: family-library-market-research,
   person-reader-prompt-cap, restart-eats-in-flight-turn, vocabulary-book`。
5. 首页截图:`KTV` 标签下挂着`粤语解说表翻译`和 `KT8-046 内容管理`,其余四张卡在核心旁边。

从说话到落盘约 5 分钟(worker 起身 + 它先读了一遍记录),人不用等在屏幕前。

### 这一趟抓到一个不是分组的 bug,而且很重

**第一次跑的时候工具被自己的门挡了**:`hi_set_home_groups is the task-manager's; role
worker does not arrange the home surface` —— 而它就是 task-manager。原因是
`X-HI-Session-Slug` 这个 header:slug 按设计保留任何文字的字母(这里就是中文标题),而
header 值必须是 ASCII,`to_str()` 直接失败,于是这个会话在服务端**根本没有身份**。
`task-manager` 是最容易中招的一类 —— 它服务整个账本,所以不带 subject,slug 只能从标题
来,而标题是中文。

代价不止是这个门:`hi_send_message` 同样要求身份,所以**一个中文名字的 worker,报告发不
回去**。那一趟的 manager 自己查出了这件事,然后绕过工具、直接手写了 `groups.json`
(写成了,但正是提示词里不许干的事)。

修的是传输不是地址:header 里 percent-encode,读的时候解回来;纯 ASCII 的 slug 编码后
原样不变。修完重跑,同一句话、同样是中文名字的 manager,工具调用 `completed`。

### 另外看过的

- 坏掉的 `groups.json`(半截 JSON)→ 端点答 `{"groups":[]}`,日志一条 WARN,界面上没有
  错误卡;换回好文件立刻恢复。
- 同名两个组被拒绝(`two groups are both called 同名`),**拒绝之后磁盘上还是上一版**。
- 不存在的 subject 被丢掉并在回执里点名。
- Cognition 自己开的任务没有被分组 —— 当时没有任何标准,按设计就该留在组外。

### 没看过的

- **Case D 整条没在活实例上走过**(2026-09-17 加的)。看过的只有两段:默认图本身是
  `hi_text_to_image` 画的;从它用 `hi_image_to_image` 改出盆栽、耳机、书三张,风格确实
  一致。task-manager 读回执、自己挑物体、画完再写回这一段,没有亲眼看过。

- **Case B**:说过一次之后,下一次整理时新任务是否真的照着 `grouping.md` 落位。
- **task-manager 的报告回到 Cognition**:这一趟它调完工具就接着去处理账本上的别的事了,
  我没等到它的 report。身份那条链已经通了(同一个 `slug` 两处共用),但这一段没亲眼看过。
- 默认分组的质量:一个完全没说过话的实例,manager 分出来的第一版像不像人心里的样子。
- 一个观察,不确定是不是问题:**这次要求本身也变成了一条任务**(`把粤语解说表和 KT8-046
  并成首页的 KTV 组`,`doing`),于是首页上多了一张卡。人确实提了要求、这确实是欠着的事,
  但一次排布调整值不值一行账,下次实测要看它有没有被正常关掉。
