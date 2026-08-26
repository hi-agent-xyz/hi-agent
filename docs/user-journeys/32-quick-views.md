# 基础信息先给一个 Quick View,复杂了再认真做

**Persona:** 同一个用户,平时随口要看一些项目、任务、账单、联系人或文件的基本信息;
知道 agent 可以把内容摆到屏幕上,但不希望每次都等一套完整的视觉设计。
**Goal:** 当普通的现成组件已经足够时,快速得到一个清楚、能用、可继续修改的 view;
只有信息真的复杂时,才投入定制设计。
**Preconditions:** host 已安装可直接 import 的 shadcn 组件源码;view 仍然是普通
JSX/HTML 文件,已有的 `hi_show`、`hi_review_view` 和 view 工具箱正常工作。

## Steps & expected UX

## 这条线画在"用什么做"上,不画在"问题重不重要"上

判断由 **view-builder 做**,不由 agent 做:agent 只知道用户问了什么,builder 才知道
手上的材料和现成组件能不能扛住。两个测试都能在写第一行之前跑完:

- **布局测试**:整页只有一次排布决定 —— 单个元素铺满、一个 flex(行或列,左右分栏
  就是它加一个固定比例)、或一个 grid。cell 或 `Card` 内部的普通堆叠不算。布局套
  布局、重叠分层、绝对定位、特殊比例,都是第二次排布决定,那就是 Custom。
- **组件测试**:Quick View 只从 quick set 组合 —— `Card`、`Table`、`Badge`、
  `Separator`、`Progress`、`Avatar`、`Alert`、`Skeleton`、`Label`、`Button`、
  `ScrollArea`、`Tooltip`。`Tabs`/`Accordion` 会藏内容(那是信息结构决定),表单类
  `Input`/`Textarea`/`Checkbox`/`Switch`/`Select` 会收集输入(那要带上校验、提交、
  pending 和错误态)—— 都不在其中。**Quick View 负责呈现和触发,不负责收集。**

一句话的检查:**先列 import**。全部来自 quick set、`react` 和语义化 HTML,且整页只有
一次排布 —— 那就是 Quick View。一旦需要自己画的东西(图表、示意图、压字的照片、可挑选
的画廊、距离代表时长的时间轴、点击以外的交互),就是 Custom。拿不准时按 Quick 走。

### Case A · 基础信息 → 直接给 Quick View

1. **用户说**:"把这个项目的负责人、状态、截止日期和最近三条任务摆出来。"
2. **Builder 判断**:一个 flex 列 + `Card`/`Badge`/`Separator`/`Table` 就够,两个测试
   都通过;它直接走 Quick View,不把"Quick View"这个内部判断说给用户听。
3. **Builder 写普通 JSX**:从已有 view 工具箱看一眼相近的旧件,必要时从头写一个
   `.jsx`;从 `@/components/ui/card`、`@/components/ui/table` 等标准路径直接 import
   `Card`、`Badge`、`Table`、`Separator`、`Progress`、`Button`。不写 JSON schema,
   不发明 DSL,不搭运行时 renderer,也不经过自定义 UI wrapper。
4. **Builder 做最低限度的交付检查**:确认数据真的落位,浅色/深色都能读,窄一点的
   frame 不溢出,空数据和错误不会变成一块空白;然后用 `hi_review_view` 看实际截图。
5. **Agent 展示并简短说明**:view 很快上屏,信息按"项目结论 → 关键事实 → 任务列表"
   排好;口头只补充屏幕上不适合放的上下文,不复述整张表。

### Case B · 用户在同一个 Quick View 上继续问

1. **用户说**:"只看还没完成的任务"或"把截止日期换成今天的最新值。"
2. **Agent 保留现有 view 的结构和 ref**,在同一个 JSX 文件里改数据过滤或文字;如果
   只是同一份内容再看一次,直接复用已有 ref,不重新 build。
3. **用户看到**:页面的布局和视觉语言保持稳定,只改变了真正需要改变的内容;不因为
   一次小改动重新做一套设计。

### Case C · 信息变复杂 → 从 Quick View 升级

1. **用户继续说**:"再比较过去六个月的趋势,标出异常,允许我点进去看每周细节。"
2. **Builder 判断**:图表没有现成组件、下钻超出点击、异常标注要自己画 —— 组件测试直接
   判 Custom;它不硬塞进原来的基础表格,而是把现有 JSX 和 ref 当作起点,**就地升级**,
   转入 Custom View 的研究、构图、交互和多轮 review。
3. **用户感受**:升级发生在原有内容之上,项目名、状态和已有事实仍在;只是为了新的
   问题增加了真正必要的可视化和交互,而不是把一个简单页面永久复杂化。

## Expected outcome

- 基础问题可以很快得到清楚的屏幕答案,等待时间主要花在拿到数据,而不是重复搭 UI。
- Quick View 是普通、可读、可维护的 JSX;后续工程师可以直接编辑它,不需要先学另一种
  描述语言。
- 现成 shadcn 组件带来一致的状态、键盘操作、主题适配和触摸尺寸;复杂信息仍有明确
  的 Custom View 逃生路径。
- 同一类信息被再次请求时,优先复用已有 ref 或旧 JSX,速度和一致性一起积累。

## Edge cases & failure modes

- **组件表达不了真正的问题** → 不强行堆卡片;升级为 Custom View,必要时新增一个经过
  review 的专用组件。
- **数据为空或请求失败** → Quick View 显示明确的空状态或错误状态及可执行的恢复动作,
  不显示一个看似成功的空白页面。
- **用户只是要上次那一份** → 直接 `hi_show(ref)`;不能把旧快照误当成"现在的状态"。
- **用户要最新数据** → 先刷新数据,再在旧 JSX 上更新内容;不能只复用旧 ref。
- **窄窗口或暗色主题读不清** → 先减少内容、改布局或拆 view,不能把字体缩到不可读。
- **现成组件缺失** → 用 shadcn CLI 安装并 review 对应组件源码,再把它加入简短可用清单,
  同时明确它算不算 quick set(呈现型、自身完备的算;藏内容或收输入的不算);不要在每个
  view 里复制一套私有按钮、表格或弹窗实现。

## UX principles this journey establishes

- Quick View 是**软引导**,不是模式锁定、协议或 DSL。
- 这条线画在**做视图要用到什么**上(一次排布 + quick set 组件),不画在问题重不重要、
  用户在不在乎上;两个测试写第一行之前就能跑完。
- **判断属于 builder**,不属于 agent:agent 手上只有用户的话,builder 手上才有材料和
  组件。给 agent 一个 quick/custom 开关就把软引导变成了模式锁定。
- Quick View 有**自己走得完的流程**:一句话说清回答什么问题、写 JSX、一次
  `hi_review_view`、交 ref、结束。构图分类、rough-then-refine、refine pass 和表现力
  标准都属于 Custom;写在同一页上,但不欠 Quick View。
- 组件准备好,才能让"快速"来自组合,而不是来自降低可用性。
- Quick View 仍然要过真实渲染、主题、窄屏、空态和错误态检查;快不等于粗糙。
- 拿不准按 Quick 走:太小了可以就地升级(同一个文件、同一个 ref),而为四条事实做了
  一整套设计,花掉的时间要不回来。
