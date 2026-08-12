# 画张图,再改一版(生成即产物,存进 drive 拿 ref)

**Persona:** 老板随口要一张图 —— 海报、示意图、给孩子的卡片;不关心用哪个模型,只看结果。
**Goal:** agent 真用图像模型画出来(不是用代码画的),存成**不会褪色**的产物,能拿去改、拿去看、拿去上屏。
**Preconditions:** 配了图像 provider(xiaoyuanzhu 模式开箱即有;BYOK 粘一把 key)。生成工具只发给 **worker** —— Reaction 只说话,产物是干活那一层的事。

## Steps & expected UX

1. **"画只戴围巾的橘猫"** → Reaction 派个 worker;worker 调 `text-to-image`,**自己挑模型**(工具描述里现列着这个账号当下能用的模型 + 哪个最好/最快/最便宜),回来的是 `⟨ref: drive/generated/<日期>/<时分秒>-戴围巾的橘猫.png⟩` 加绝对路径和 URL。
2. **worker 把 ref 报给 Reaction** → Reaction 说一句 + `show` 一个内嵌 `<img src="/api/drive/file/…">` 的 view,图**真出现在屏幕上**。
3. **"围巾换成红的"** → 同一个 ref 交给 `image-to-image`,原图不动,回来一个**新的 ref**;再 show 一次,原地替换。
4. **"手机壁纸尺寸"** → `size` 是可选旋钮之一,agent 自己填;填了模型咽不下的值(gpt-image 要求边长 16 的倍数)→ 当场报错**告诉它该怎么填**,不是甩一个 400。
5. **"让它动起来"** → 那张图的 ref 交给 `image-to-video`;工具**立刻返回**,不占线;几分钟后片子好了,以**一条消息**带着 `⟨ref: …⟩` 送回发起的 worker。

## Expected outcome

- 图是**图像模型画的**,不是 PIL/SVG 糊出来的(这正是 [12](12-play-with-child.md) 实测记下的缺口)。
- 产物落在 `drive/generated/`,**永久**;raw 里的相机帧会随天冷掉褪色,画出来的东西不会。
- 一个 `ref` 语法通吃三种来源:相机拍的、别人递的、自己刚画的 —— `image-to-image` / `image-to-video` / `image-text-to-text` / `show` 都吃同一个参数。
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

工具层跑通,**agent 自主那一段没测到**——这台实例的 Reaction 压根没开口(`turns_total: 0`,日志 `turn wrote a reply and said none of it`),没派 worker,生成工具一次也没被调到。那是本次改动之外的既有问题。于是直接以 worker 身份打 `/mcp`,验的是工具层本身:

- ✅ **`text-to-image` 真出图**:Doubao seedream 回来 2048×2048 JPEG(245 KB),落 `drive/generated/2026-08-12/084941-an-orange-cat-wearing-a-red-scarf.jpg`,`/api/drive/file/…` 200 + `content-type: image/jpeg`。文件名带提示词、扩展名 **由字节嗅出来**(没要 png 它给 jpeg,如实记 jpg)。
- ✅ **菜单进了工具描述**:`tools/list` 里 `model` 写着 "Reachable now: doubao-seedream-5.0-lite. Omit to use doubao-seedream-5.0-lite."
- ✅ **`image-to-image` 该拒就拒**:seedream 这条线没实现编辑 → "editing is not implemented for doubao-seedream-5.0-lite (the doubao wire) — name a gpt-image model instead",不静默换模型。
- ⚠️ **gpt-image-2 只有单测,没实跑**:这个账号 broker 菜单里只有 seedream,手上也没有 OpenAI key。请求形状、size 规则、透明底拒绝、multipart 编辑都有单测,**真调用未验**。

### 修掉的两个真 bug(都是实测才炸出来的)

1. **`image-text-to-text` 不认 drive ref** —— 它在 resolve 之前先用 `parse_ref` 推 MIME,而那只认 channel ref,于是"看看你刚画的那张"直接报 malformed。改成先 resolve、再从字节嗅 MIME,和生成工具走同一条 `read_ref`。
2. **网关的 wire 盖过了模型** —— adapter 原来在 init 时按 provider 定死;songguo 一个 `openai-images` wire 后面同时供 seedream 和 gpt-image,于是 seedream 的编辑被当成 OpenAI multipart 发出去,网关回 "could not parse the JSON body"。改成 **按调用、按模型名定** adapter,wire 只在模型名不认识时兜底。

### 待复测

- Reaction 开口 / 派单那段通了以后,整条"老板说一句 → 图上屏"要重跑。
- gpt-image-2:等 broker 菜单里有,或粘一把 OpenAI key。
- 视频两条(`text-to-video`/`image-to-video`)以及"几分钟后消息送回"那条回路,一次没跑过。
