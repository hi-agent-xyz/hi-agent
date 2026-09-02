# 在别的 app 里按一下侧键,它就看见我在看什么

**Persona:** 用户正在别的 app 里(地图、邮件、一份 PDF、一条报错),想就眼前这一屏问一句。
**Goal:** 不走"截屏 → 存相册 → 切到 hi agent → 找那张图 → 上传 → 再打字说是什么",按一下就完事。
**Preconditions:** iPhone 已配对一个 core;快捷指令 `Take Screenshot → Show My Screen` 已绑到操作按钮(或轻点背面 / 控制中心)。是 [18](18-send-files-to-agent.md)/[19](19-upload-passport.md)"递一个物件"的**手势版**,也是桌面 ⌘⌘ "come and see this" 在手机上的同一件事。

## Steps & expected UX

1. **在任意 app 里按下操作按钮** → 屏幕不跳走,截图当场拍下(拍的是**那个 app**,不是 hi agent)→ hi agent 打开,底部一条"正在把你的屏幕给它看…";落地后变"已给 <core 名字>",两秒自己收走。
2. **对话里是两条自己的消息**:一句 "Here's my iPhone screen right now.",和那张图 —— 一次到达,一件事一条消息,不是一句把图裹进去的旁白。
3. **agent 醒过来就图说事**:Reaction 只读到那句话(它不开文件),据此判断这是"当下的屏幕、人要我看"→ 交给 Cognition 打开图 → 回的是屏幕上那件事(这条报错怎么修 / 这家店几点关),不是"你发来一张图"。
4. **想连着说点什么** → 在快捷指令里给 Show My Screen 填 Note("这段报错什么意思")→ 那句话**代替**默认那句,仍旧排在图前面。
5. **再按一次** → 新的一屏接着进同一条对话,不新开话头。

## Expected outcome

一次按键 = 一次"你看这个"。全程不打字、不进相册、不切 app 找东西;人回到 hi agent 时,该说的话和该看的图已经在对话里了。

## Edge cases & failure modes

- **还没配对任何 core** → banner 直说"这台设备还没配对,没人可看",截图**留在手上**,配好后"再试一次"就发出去,不用重按一遍。
- **core 不在 / 网断** → 同上:一条能读懂的原因 + 再试一次。
- **凭证没了(core 那边解绑)** → "这台设备已不再和 <core> 配对,请重新配对",不是一个 HTTP 数字。
- **首次运行** → iOS 自己会问"允许这个快捷指令截屏吗",一次性,和本 app 无关。
- **刚开机还没解锁过** → Keychain 是 `afterFirstUnlock`,凭证读不到 → 走"再试一次"那条路,不静默失败。
- **截到密码 / 银行界面** → 是人自己按的键,和把图拖进对话框是同一件事,不额外设闸(soft guidance,连 [privacy](../arch/privacy.md))。
- **没有操作按钮的机型** → 同一条快捷指令挂到轻点背面 / 锁屏按钮 / 控制中心,行为一样。
- **iPad** → 那句话会说 "my iPad screen";除此之外一样。

## Open questions

- 把签好名的 `.shortcut` 挂到 hi.xiaoyuanzhu.com,让"建快捷指令"的三步塌成一次导入 —— 需要一台 Mac 签名 + 一个地址托管,现在没做,所以 app 里是四步说明书。
- 桌面那句是 "Here's my screen right now.",手机这句多带了机型。两句要不要收敛成一句,取决于"是手机屏"这件事对怎么看图有没有用。
- 手势只有"看这一屏"。桌面还有按住 ⌘ 的持续注意([15](15-talk-over-the-agent.md) 那路),手机上对应的东西是什么,没想。

_机制:iOS 上**任何 app 都不能给别的 app 拍照**,能拍的只有系统自己 —— 快捷指令的 Take Screenshot 动作,而操作按钮跑快捷指令时不离开当前 app。所以图由系统拍,`ShowScreenIntent`(App Intent,`openAppWhenRun`)只是接过来的载体:它把字节 POST 给 `POST /api/in/file`,并在 multipart 里带一个 `note` 部分说这是什么 —— 和桌面 ⌘⌘ 在进程内填的是同一个字段(`files::ingest_file` 的 `note`)。截图必须先于开 app 拍好,否则拍到的是 hi agent 自己。_

_状态:Rust 那半有集成测试(`tests/transcript.rs::a_carriers_note_precedes_the_file_it_frames`);iOS 那半 `make ios` 编译通过,**没在真机上跑过** —— 操作按钮和 Take Screenshot 都只有真机有,模拟器没有,所以第 1 步至今没人看着它发生。_
