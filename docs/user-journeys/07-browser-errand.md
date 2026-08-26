# 用浏览器替我办事

**Persona:** 用户把一件"得在网页上点点点"的事交给 agent(查询、比价、填表、下单前的准备)。
**Goal:** agent 真的打开浏览器、实操完成,而不是嘴上说怎么做。
**Preconditions:** 在线;有浏览器 effector(Playwright/CDP),由 worker 的 Bash/code-exec 驱动。effector 缺失时**置备它本身是任务的一部分**(范型见 [02](02-feishu-sprint-backlog.md))。

## Steps & expected UX

1. **"去这个网站帮我查一下 X / 把这个表填了"** → 接住;若首次需置备浏览器工具,先请示安装再上手。
2. **实操** → 打开页、导航、点击、取数/填写;过程可简短汇报关键节点,不逐帧直播。
3. **结果** → 把拿到的结果/截图交给用户看;**怎么开这类页**沉淀成可复用技能。
4. **涉及账号/支付/敏感提交** → 到"只有用户能做 / 该用户拍板"的步骤**停下来请示**,不擅自提交。

## Expected outcome

- 事**真的被办了**(页打开了、数取到了、表填好了),不是一段操作说明。
- 不可控处(验证码、登录墙、二次确认)如实降级或请示,不硬闯。

## Edge cases & failure modes

- 验证码 / 人机校验 → 请用户介入那一步,不绕过。
- 页面改版 / 选择器失效 → 重试一次;仍不行如实报,不假装成功。
- 敏感动作(付款、删除、对外发送)→ 默认停下请示([[careful-irreversible-actions]])。

## Open questions

- 哪些动作算"敏感、必须先问"——给个软清单还是逐次判断?
- 实时浏览画面要不要也作为一个 view 给用户看?

_机制:技能(怎么开这类页)+ effector(浏览器)。可行性:**可行**。成熟度:依赖浏览器 effector + 技能层(未建)。_

## 实测 2026-06-18 · origin/main 0f68aaf

- ✅ 诚实:"我没有浏览器工具,没法直接打开和操控网页",并给降级(用搜索查),不假装操作页面。
- ⚠️ **没主动提议置备浏览器 effector**(按 [02](02-feishu-sprint-backlog.md) 范型"缺工具是任务的一部分",应研究 + 请示装),而是直接降级到搜索。浏览器 effector 未建,真实操作未测。

## 复测 2026-08-26 · 隔离实例 `--data-dir /tmp/hi-tools2`

浏览器现在是工作间里的一条**工具笔记**:`skills/factory/browser.md` 带 `purpose:`/`use: browser`,
`<data_dir>/bin` 进了每个 session 的 PATH,`bin/browser` 在**调用时**才解析这台机器上的
Chrome(所以从不开网页的机器不会白下 100 MB)。

- 🔴 **第一次跑,暴露的是老毛病:指令挂在了不在路径上的那一节。** "去看下这个页面"这类活,
  Cognition **自己**用 `curl … | sed -n '1,90p'` 办了,**一个 worker 都没建**,工作间从没被扫过,
  结果把 HN 第二条当成了第一条。扫描规则当时只写在 `general.md`(worker 的 prompt)里。
  修复:Cognition 手上有 codex 自带的 shell,规则同时写进 `cognition.md`,由
  `identity::tests::a_worker_scans_the_workshop_before_saying_it_cannot` 钉住。
- ✅ **修完之后,发现—读笔记—用工具整条链路实测通过。** 纯文本页面上它先
  `grep -rn "^purpose:"` 扫工作间、`sed` 打开 `browser.md` 再动手,然后**正确地用了 `curl`**
  ——笔记本身就说"页面只是文本时,普通抓取才是对的工具"。换成前端渲染的页面
  (`hi-agent.xyz`,`curl` 只拿到 789 字节空壳),它先试 `curl`、看出是空的,
  **再落到 `browser --dump-dom`**,把 4 条 FAQ 原文一字不差取了回来。
  便宜的路走不通才伸手拿工具,正是笔记要的那个次序。
- 🟠 **本 journey 的正题仍未测:点、填、多步操作。** 两次都是"读一个页面",而且两次
  Cognition 都自己干了、没派 worker——prompt 里"真正的差事仍旧交给 worker"这句写了但没被验证。
- 🟠 **缺工具时主动置备**依然未测(见上一次实测的同一条),那是工作间的"写"那一半,尚未建。
