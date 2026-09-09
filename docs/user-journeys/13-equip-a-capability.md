# 给我配一个外部能力(厂商 API + 凭证)

**Persona:** 用户 / 开发者给 agent 装一个外部能力(比如一个厂商人脸检测 API),给它端点和密钥。
**Goal:** agent 把"有这能力 + 怎么用"记进可按需取用的认识,把**端点 / 调用法 / 密钥位置**逐字记进 drive 笔记本,且**密钥永不进脑子**。
**Preconditions:** 用户能提供端点 + 凭证;有 drive 笔记本(`drive/notes`)与"调用时由 effector 解密"的约定。**底层模型见 [data-dir-layout](../data-dir-layout.md) 中的 E2。**

## Steps & expected UX

1. **"给你配个人脸检测 API,端点在这、key 在这"** → agent 接收并**分流**:
   - "有人脸检测能力、适合什么、怎么调" → 记忆 / 技能(**按需加载**,不常驻占上下文)。
   - "端点 / 调用范式 / 密钥放在哪(env 变量名)" → **drive 笔记本逐字**。
   - 真正的密钥 → 放 env / drive,**笔记只记它在哪**;调用时由 effector 读取。
2. **首次调用** → 真的打通,跑出结果。
3. **日后再遇相关任务** → 自动想起"我有这能力",从笔记本取调用法直接用,无需用户重提。

## Expected outcome

- 能力真能调用;**转录 / 对话里不出现明文密钥**。
- 下次相关任务自动复用这条能力,不必重新配置。

## Edge cases & failure modes

- 密钥失效 / 额度用尽 → 自检或调用时发现 → 找用户重配那一件事(说人话、说一次)。
- 端点 / 签名方式变了 → 更新笔记本那一页(逐字),不靠"记忆里大概的样子"。

## Open questions

- 凭证落 `drive/` 还是只在笔记本里**指向** env 变量?(data-dir-layout fork,倾向后者)
- 能力多了之后,"我有哪些能力"的索引怎么默认加载而不撑爆上下文?

_机制:能力 = effector 可达(config / env)+ 按需技能(怎么用)+ drive 笔记本(逐字凭证,调用时解密)。可行性:**可行**。成熟度:依赖 drive 笔记本 + 技能层 + 调用时解密(未建)。_

## 实测 2026-06-18 · origin/main 0f68aaf

> 跑在旧布局上:文中的 `self.md` 已删除(见 [`docs/memory.md`](../memory.md))。但下面那条 🟠 的**发现**没有过期——密钥永不进脑子,笔记只记它在哪(env 变量名);今天等价的落点是 facet 与 generated prompt,规矩一样。

- ✅ 没把密钥念出口(口播只说"记下了,以后用这个端点");能力知识(端点/用法)有记录。
- 🟠 **密钥进了脑子**:明文 `sk-fake-…` 被写进 self.md。违"密钥永不进脑子,笔记只记它在哪(env 变量名)"。根因:drive 笔记本/调用时解密未建,agent 把一切(含密钥)塞进 self.md。修复方向 = 建 drive 笔记本 + 一条 prompt 硬规矩(secret 永不入 memory 文件,只记 env 变量名)。

## 复测 2026-09-09 · mcp-as-skill —— 只测 MCP 这一种形状

先是同一天早上的真实失败,它才是这次改动的起因。用户在文字频道贴了一台远程 Android 的 MCP 端点和 bearer token,说"你可以试试能不能用"。当时的形状是 `hi mcp <endpoint>` 这条命令,而**它没有任何地方能放一个 header**:cognition 14:47 跑 `hi mcp https://abacad.ai/mcp list`,拿回 `401 missing bearer token`(`data/memory/raw/sessions/fcd9520cc9a1/cognition.jsonl`)。它随后绕开产品路径,手写 python + `urllib` 打通,靠的是这台机器 shell 里恰好 export 了 `ABACAD_KEY`。**成功了,但不可复用**:什么都没注册,换台机器就没了。

改后的形状:MCP server 是一篇 skill(front matter 里的 `mcp:` 端点 + `mcp_auth:`),**由派活的 rung 在 `hi_create_worker` 上点名,挂进那一个 worker 的线程**。

- ✅ **对 `https://abacad.ai/mcp` 真实拿到过工具清单**:`list_devices` / `screenshot`(带 accessibility 树)/ `tap` / `swipe` / `input_text` / `back` / `home` / `recents`,以及桌面端 `click` / `press_keys` / `composite`、浏览器 `execute`、`send_file` / `get_file`、`screen_recording`。**但这次观察走的是中途被推翻的那条 proxy 路径**(hi-agent 自己发 JSON-RPC),不是现在这条 attach 路径。端点、凭证和这台 server 的真实能力是实的;**现在这条链路的实测是零。**
- ✅ 注册就是写一篇 skill,`mcp:` 解析、按名字查、名字对不上跳过而不致命、非 URL 端点当命令 spawn——都有单测(`a_named_server_is_attached_to_the_thread_that_asked_for_it`)。
- ✅ **没点名的活一个 server 都不带**,有断言钉住。这是"哪些 session 拿到哪些工具"的全部答案:不是 role 表,不是存下来的等级,是派活那一刻的选择。
- 🟠 **完整链路从没跑过。** 没有人看过 cognition 派一个 worker 去注册、报告回来、再派第二个带着 `servers:` 的 worker 真的点亮那台手机的屏幕。从"人贴一个端点"到"屏幕被点了一下"这条路**一次都没走完**。
- 🟠 **注册的那个 worker 自己用不了它。** 线程的工具在打开时定死,所以写下 skill 的 worker 没有任何调用可以验证端点和密钥是对的——第一次真实调用发生在下一次派活。这直接顶撞 `equipping-a-tool.md` § 6"没跑过就别写下来"那条规矩,是 attach 相对 proxy 唯一更差的地方。
- 🟠 **2026-06-18 那条 🟠 只算部分解决。** 那次的病是明文 key 被写进 `self.md`(模型的记忆文件),根因"没有别的地方可放"。现在 key 有了确定的落点(skill 的 front matter),host 读它、干活的 rung 不读那个文件——但它**仍然是一个明文文件**,只是不再进记忆。中间我建过一张单独的密钥表把它藏得更深,被判定为过度设计删掉了:人和 host 都是可信的,而 key 本来就是人明文贴进一个逐条留存的频道的。
- 🟠 **一次调用一个回答**,progress / sampling 收不到。abacad 的 `screen_recording` 正是价值在流上的那种,只能 start / stop / poll。

_机制:MCP server = 一篇带 `mcp:` 的 skill(注册)+ `hi_create_worker { servers: […] }`(组织)+ `thread_config` 写进 codex 的 `mcp_servers`(加载)。可行性:**未验证**——真实 server 的能力已证实,这条链路未走通。成熟度:有单测,零实测。_
