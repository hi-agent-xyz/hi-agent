# 画张图,再改一版(生成即产物,存进 drive 拿 ref)

**Persona:** 老板随口要一张图 —— 海报、示意图、给孩子的卡片;不关心用哪个模型,只看结果。
**Goal:** agent 真用图像模型画出来(不是用代码画的),存成**不会褪色**的产物,能拿去改、拿去看、拿去上屏。
**Preconditions:** 配了图像 provider(xiaoyuanzhu 模式开箱即有;BYOK 粘一把 key)。生成工具只发给 **worker** —— Reaction 只说话,产物是干活那一层的事。

## Steps & expected UX

1. **"画只戴围巾的橘猫"** → Reaction 派个 worker;worker 调 `hi_text_to_image`,**自己挑模型**(工具描述里现列着这个账号当下能用的模型 + 哪个最好/最快/最便宜),回来的是 `⟨ref: drive/generated/<日期>/<时分秒>-戴围巾的橘猫.png⟩` 加绝对路径和 URL。
2. **worker 把 ref 报给 Reaction** → Reaction 说一句 + `hi_show` 一个内嵌 `<img src="/api/drive/file/…">` 的 view,图**真出现在屏幕上**。
3. **"围巾换成红的"** → 同一个 ref 交给 `hi_image_to_image`,原图不动,回来一个**新的 ref**;再 show 一次,原地替换。
4. **"手机壁纸尺寸"** → `size` 是可选旋钮之一,agent 自己填;填了模型咽不下的值(gpt-image 要求边长 16 的倍数)→ 当场报错**告诉它该怎么填**,不是甩一个 400。
5. **"让它动起来"** → 那张图的 ref 交给 `hi_image_to_video`;工具**立刻返回**,不占线;几分钟后片子好了,以**一条消息**带着 `⟨ref: …⟩` 送回发起的 worker。

## Expected outcome

- 图是**图像模型画的**,不是 PIL/SVG 糊出来的(这正是 [12](12-play-with-child.md) 实测记下的缺口)。
- 产物落在 `drive/generated/`,**永久**;raw 里的相机帧会随天冷掉褪色,画出来的东西不会。
- 一个 `ref` 语法通吃三种来源:相机拍的、别人递的、自己刚画的 —— `hi_image_to_image` / `hi_image_to_video` / `hi_image_text_to_text` / `hi_show` 都吃同一个参数。
- 文件名带提示词,一年后翻 `drive/` 还认得出哪张是哪张(中文提示词给中文文件名)。

## Edge cases & failure modes

- **没配 provider** → 说"没配图像 key,去 Settings",不说别的;这和"这个模型不会改图"是两码事,后者要报**哪个模型会**。
- **点名一个谁都不提供的模型** → 列出当下能用的,**绝不悄悄换一个**代生成 —— 换了会被记成"就是你要的那张"。
- **旋钮这条线咽不下**(seedream 不吃 `quality`/透明底;gpt-image-2 不吃 `watermark`、也**没有**透明底)→ 报错时点名换哪个模型能办,不做静默丢弃。
- **视频跑了一半 worker 结束了** → 片子照样存进 drive,日志记一笔;不假装没干过这活。
- **视频十五分钟没动静** → 送回一条"还没完成,可能稍后才上游落地",不无限等。

## Open questions

- 生成这件事要不要进 journal?现在**不进** —— worker 把产物报给 owner,那条 report 本来就落在 `worker` 频道,事件已经在记忆里了。要是以后想按"我画过什么"检索,再单独说。
- `drive/generated/` 长期只涨不减(遗忘是 keep 偏向的,drive 不褪色)。到多大才需要管,现在不知道。
- 多图(`n>1`)现在只有 gpt-image 那条线支持;seedream 要多张得多调几次。

_机制:`image_gen`/`video_gen` 两个能力 + `drive/` 产物家 + `drive/<path>` ref 语法;模型由 agent 选,wire 是内部管道、**没人在 Settings 里挑**。_

## 实测 2026-08-12 · feat/image-generation(Mac mini,`--data-dir /tmp/j30`,xiaoyuanzhu 开箱账号)

工具层跑通,**agent 自主那一段没测到**——这台实例的 Reaction 没派 worker,生成工具一次也没被调到(`turns_total: 0`)。那几轮 turn 只写了文字、没有调 `hi_say`,但那是设计内的沉默,不是故障;缺的是委派与调用。那是本次改动之外的既有问题。于是直接以 worker 身份打 `/mcp`,验的是工具层本身:

- ✅ **`hi_text_to_image` 真出图**:Doubao seedream 回来 2048×2048 JPEG(245 KB),落 `drive/generated/2026-08-12/084941-an-orange-cat-wearing-a-red-scarf.jpg`,`/api/drive/file/…` 200 + `content-type: image/jpeg`。文件名带提示词、扩展名 **由字节嗅出来**(没要 png 它给 jpeg,如实记 jpg)。
- ✅ **菜单进了工具描述**:`tools/list` 里 `model` 写着 "Reachable now: doubao-seedream-5.0-lite. Omit to use doubao-seedream-5.0-lite."
- ✅ **`hi_image_to_image` 该拒就拒**:seedream 这条线没实现编辑 → "editing is not implemented for doubao-seedream-5.0-lite (the doubao wire) — name a gpt-image model instead",不静默换模型。
- ⚠️ **gpt-image-2 只有单测,没实跑**:这个账号 broker 菜单里只有 seedream,手上也没有 OpenAI key。请求形状、size 规则、透明底拒绝、multipart 编辑都有单测,**真调用未验**。

### 修掉的两个真 bug(都是实测才炸出来的)

1. **`hi_image_text_to_text` 不认 drive ref** —— 它在 resolve 之前先用 `parse_ref` 推 MIME,而那只认 channel ref,于是"看看你刚画的那张"直接报 malformed。改成先 resolve、再从字节嗅 MIME,和生成工具走同一条 `read_ref`。
2. **网关的 wire 盖过了模型** —— adapter 原来在 init 时按 provider 定死;songguo 一个 `openai-images` wire 后面同时供 seedream 和 gpt-image,于是 seedream 的编辑被当成 OpenAI multipart 发出去,网关回 "could not parse the JSON body"。改成 **按调用、按模型名定** adapter,wire 只在模型名不认识时兜底。

### 待复测

- Reaction 开口 / 派单那段通了以后,整条"老板说一句 → 图上屏"要重跑。
- gpt-image-2:等 broker 菜单里有,或粘一把 OpenAI key。
- 视频两条(`hi_text_to_video`/`hi_image_to_video`)以及"几分钟后消息送回"那条回路,一次没跑过。

## 复测 2026-08-31 · 主实例(本机 `data/`,xiaoyuanzhu 开箱账号)

**没重跑,是翻一轮真发生过的**:老板正看着 `factory/tools`,问 "what models do you have for gen/edit image?"。整轮帧日志里,四个生成工具一次没被碰过。

- ❌ **没派 worker**:Reaction 09:20:06 把问题递给 Cognition,brief 里已经写死了方向 —— "inspect the local imagegen skill/provider configuration";Cognition 09:21:04 直接开 shell 去 grep `data/codex-home/skills/.system/imagegen`,读的是 **codex 自带的系统 skill**(8-28 随 runtime 自动装进 `CODEX_HOME`),不是自己的工具。两层都没想到派个 worker 去读它自己的 `tools/list`。
- ❌ **于是答案是别人的**:回给老板的是"内置 `image_gen`(模型名不对外暴露)、`gpt-image-2`、`gpt-image-1.5`" —— 逐句出自那个 skill 的 `SKILL.md` / `references/cli.md` / `references/image-api.md`。它甚至如实说了 `gpt-image-1`/`-1-mini`"只出现在参考文本里",却把同一份文本里的其余型号当成了本机可用。本机真打得通的 `doubao-seedream-5.0-lite` 一个字没提;`gpt-image-1.5` 则根本不可达(那条 CLI 要 `OPENAI_API_KEY`,`child_env` 不给)。
- **根因不在 image_gen,在提示词**:`hi_text_to_image` / `hi_image_to_image` / `hi_text_to_video` / `hi_image_to_video` / `hi_video_text_to_text` 五个工具,**任何一份 prompt 都没提过一个字** —— 只有 worker 的 `tools/list` 里有。会说话的那层不知道自己有手,就去找了别人的手。这也解释了 2026-08-12 待复测里那句"Reaction 没派 worker":不是不肯派,是不知道有什么可派。
- ✅ 已改:`reaction.md` 加 "What the rest of you can make"(四件事 + 常见说法 + "不是菜单、也不是能力边界"),`cognition.md` 加"别去研究自己的身体 —— 谁拿着工具谁才看得见当下可达的模型",`workers/general.md` 补上四个生成工具与 drive/ref 那条链;顺带修掉一个早不存在的工具名(`watch` → `hi_video_text_to_text`),并加了个测试:prompt 里写的每个 `hi_` 名字都必须是真声明过的工具。
- ⏳ **整条链路仍未实跑**:2026-08-12 的三条待复测(老板一句话 → 图上屏、gpt-image-2 真调用、两条视频)一条都没动;这次只是把"派不出去"的原因拆掉了。
