# 在任务上直接回一句

**Persona:** 手上有几件事在等他的人。看板上一张卡写着 *等你处理*:"赵力 must open
http://127.0.0.1:7788, press Run, listen to both takes, and reply ACCEPT or REJECT"。他听完了,
想说"ACCEPT,第二版更自然"。
**Goal:** 就在这张任务里回这一句,不用回到对话里再说一遍"关于试听音色那个……";回完之后
这张卡不再冲他喊,活儿自己接着走。
**Preconditions:** `factory/tasks` 正常;这一行 `doing`,最新一行是 `waiting`。

## Steps & expected UX

1. **他打开这张卡**(看板上点卡,或者从首页的 *等你处理* 跳到看板再点)。面板顶上是
   *等你处理* 那一行原话,链接可点;面板底部、状态按钮上面是一个回复框,占位字写
   "回复这个任务"。
2. **他打字,回车。** 回车先让给输入法(拼音选字的那个回车不会把半截字母发出去);
   Shift+回车换行。框里有字时按 Esc 清掉草稿,空框按 Esc 才关面板。
3. **框清空,面板当场重读记录**:时间线最上面多一行 *你的回复 · ACCEPT,第二版更自然*,
   *等你处理* 那块消失;看板卡片、首页卡片上的 *等你处理* 跟着消失——三处读的是同一个
   `latest`。
4. **对话里也有这句话**,气泡上方一行小字 `↩ 试听 TTS 新音色`,点它打开任务看板。它在对话里
   是因为它就是他说的一句话,而说过的话只有一个列表。
5. **Reaction 一般不出声。** 他已经看见这句话落在那一行上了,"收到"不带任何他不知道的信息。
   有话可说(或者这句话里问了它什么)才说。
6. **Cognition 收到的交接里带着 `⟨on task: try-the-new-voice — 试听 TTS 新音色⟩`**——是哪一行
   是入口给的事实,不是 Reaction 的猜测。它把这件事送到该去的地方:这一行上有人就发给那个
   会话;这是 `waiting` 要的那个判断(ACCEPT/REJECT)就交给 `task-manager` 去裁;没人在做、
   这句话解开了卡点,就在这个 subject 上起 worker,worker 在开场的记录里就能读到这句回复。
7. **manager 看到 ACCEPT 就关这一行**,关单的 `update` 引用他的原话;如果他回的是 REJECT
   或者一个问题,这一行回到干活,而不是关掉。agent 的回答,如果有,在对话里说。

## What must be true

- **一句话,一个列表。** 回复就是 `POST /api/in/text?task=<subject>`:同样过凭证扫描、同样
  归属发送者、同样进 journal、同样交给 Reaction。没有第二个入口,也没有第二个会话。
- **"在哪说的"是入口的事实。** 只有从回复框进来的那句带 `task`;在对话里提到某个任务的
  话不带,代码不读字去猜。不存在的 subject 在读 body 之前就 404。
- **行上记一行 `replied`,由 store 写。** 它盖掉上面的 `waiting`(`moved` 不算,`replied` 算),
  并且让这句回答**活在行上**:真在等人的行上通常没人,只活在邮件里的回答会被重启吃掉。
- **行上那句是遮过密钥的。** 记录文件(`memory/tasks/<subject>.md`)会话用 shell 就读得到,不经过 prompt 那道替换;
  写进去之前先过 `mask_known`,行上留 `⟨secret: …⟩`,对话和 journal 留原文。
- **走 store 的那把锁,不过 record gate。** 和每个动词一样在同一把锁下写,同一刻落在这一行的
  `hi_task_note` 两行都在;gate 判的是 agent 写给他的话,他写回来的原话不归它判。
- **不绕过梯子。** host 不把回复直接塞给行上的会话:主场景里行上没人,回答判断是状态迁移
  (manager 的),第二条从人到 worker 的路就是第二个派活的人。

## 实测 2026-09-19(本机,scratch data dir,release 构建,**没有模型**)

故意设成 BYOK、不给 key,所以不会去 broker 开号,Reaction/Cognition 的每一轮都失败——这一趟
只看得到入口、记录和界面,看不到梯子。账本搬到 `memory/tasks/<subject>.md`、记录改由三个动词写
之后又 rebase 重跑了一遍,下面是后一遍。

- **HTTP 直打**:`POST /api/in/text?task=try-the-new-voice` 回 202;`/api/out/text` 追加的消息
  带 `"task":{"subject":"try-the-new-voice","title":"试听 TTS 新音色"}`;`memory/tasks/try-the-new-voice.md` 末尾多了
  `replied — ACCEPT，第二版更自然`,时间是消息自己的;`GET /api/tasks` 的 `latest.kind` 变成
  `replied`。`?task=nobody-filed-this` 回 404,对话里什么都没多。日志一行
  `POST /api/in/text … task="try-the-new-voice"`。
- **真浏览器走了一遍**(headless Chrome 过 CDP,1280×860):看板上卡片 *Needs you* → 点开,
  回复框在状态按钮上方 → 打字、回车 → 框清空、*Needs you* 块消失、时间线最上面是
  *you replied*(accent 色)→ 对话里那句带 `↩ 试听 TTS 新音色`。
- **密钥遮挡**只在集成测试里看过(`a_reply_typed_on_a_task_carries_the_row_and_the_row_keeps_it`):
  对话里是原文,行上是 `⟨secret: …⟩`。
- **reconcile 不把这一行当成"绕过动词的手改"**,也只在单测里看过
  (`a_reply_is_the_hosts_write_and_lands_beside_a_note`,顺带证明同一刻的 `hi_task_note` 两行
  都在):这个 pass 在建窗口时跑,没模型的实例每一轮在那之前就失败了,手改一行作对照也没
  触发,所以活实例上看不到它。

### 没看过的

- **整条梯子。** Reaction 看到 `⟨on task⟩` 是否真的不说"收到";Cognition 是否按行分派、
  是否把判断交给 manager 而不是自己关;manager 是否在 ACCEPT 上关、在 REJECT 上不关。
  要在有模型的实例上(Mac mini)对一行真的 `waiting` 回一句来看。
- **重启后。** 回完就重启,Cognition 开机那一轮是否把"最新一行是他的回复"读成"球在我们这边"。
- **真输入法。** 拼音选字的回车、Esc 取消组字,只按 `inputMethodHasKey` 的逻辑推过,没在真
  IME 下按过。
- **手机宽度**下面板底部回复框 + 按钮的排布。
- **从首页过来的那一跳**仍然落在整块看板上,不是那一行(`home.md` § Open 记着的欠账)。
