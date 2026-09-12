# 在任何 app 里点"分享",选 hi agent

**Persona:** 用户在相册 / 浏览器 / 文件 / 微信里看到一个东西 —— 一张图、一份合同、一条链接、一段选中的字 —— 想给 agent。
**Goal:** 不走"存下来 → 切到 hi agent → 找到它 → 上传",用系统分享单,一步过去,而且**落在对话里**,可以接着说。
**Preconditions:** 手机已配对一个 agent。iOS 装了 app(分享扩展随 app 装);Android 同理。
是 [18](18-send-files-to-agent.md)/[19](19-upload-passport.md) "递一个物件" 的**系统入口版**,和 [36](36-show-your-screen-from-a-button.md) 是一对:那个是手势,这个是分享单。

## Steps & expected UX

1. **任意 app 里点分享 → 选 Hi Agent** → 没有二次界面,不问发给谁、不让填说明,分享单当场收走。
2. **分享单里一屏一句话:"Ready for your agent"**,一个主按钮"Open and say something",一个"Not now"。点主按钮 → hi agent 到前台,停在对话上,底部一条"正在发送…",落地变"已发给 <agent 名字>",两秒自己收走。点 Not now → 留在原来那个 app,东西也不丢,下次打开 hi agent 自动发。
3. **对话里就是那个东西本身** —— 没有一句"用户分享了一个文件"的旁白。**分享不带 note**:分享什么就是说了什么,而人下一秒就在对话框里,那句话该他自己说。
4. **接着打字** → "这个合同第几条有问题?" / "这链接里的店几点关?" —— 这才是整件事的目的:分享是把东西放到桌上,不是发完就走。
5. **链接走的是"说"那条路** → `POST /api/in/text`,在对话里就是一行 URL,和自己打上去一模一样;不是一个要 agent 打开才知道是链接的文件。
6. **一次分享九张图** → 九条消息,一条一张;顺序是选的顺序。

## Expected outcome

系统分享单变成 agent 的一个入口。任何 app 里的任何东西,两下点击到对话里,并且**落地的那一刻人就在能说话的地方**。

## Edge cases & failure modes

- **还没配对** → banner 直说"这台设备还没配对,没人可接";东西**不丢**:iOS 留在磁盘队列上,配好后下次打开自动发。
- **网断 / agent 不在** → 同上,留着,下次进 app 再发。Android 没有队列,直说失败,要重新分享一次。
- **一个死活发不出去的东西** → 不挡后面的:队列继续往下发,坏的那个留着下次再试。代价是 banner 会一直提示 —— 那正是"这里有 bug"该有的音量。
- **超大视频** → core 这边**没有上限**,边收边落盘;iOS 这边扩展先把字节抄进 App Group(必须抄,item provider 的 URL 出了进程就失效),之后由 app 上传,慢但活得过分享单关闭。**Android 这条没关**:上传挂在 activity 的 scope 和 `content://` 授权上,传大视频时走开可能断,要补前台服务。
- **iOS 没把 app 拉起来** → 东西已经在队列上了;人自己点开 hi agent,下一次前台就发出去。**唤起 app 这一步是尽力而为,不是送达路径**(见下)。
- **分享单里出现两个 Hi Agent** → 不会:只注册了分享扩展一条入口,没有再声明文档类型。
- **旋转屏幕 / 回退重进** → Android 把 intent 的 action 清掉,同一张照片不会因为重建 activity 发第二遍。
- **别的 app 给的文件名很怪**(引号、换行、超长)→ 名字在本地重造,只留普通字符,不可能把引号带进 `Content-Disposition`。

## Open questions

- 唤起用的 Universal Link 指向 `hi-agent.xyz` 顶级域名,**而不是用户自己 core 的地址**。想让 `iloahz.hi-agent.xyz` 这种链接也直接开 app 是另一件事:每个子域名都得自己供 AASA(而子域名是隧道直通 core,得由 broker 代答),更麻烦的是**分享出去的 view 也在那些子域名上** —— 声明整个通配符会让别人发你的 view 链接打开你的 app、指着他的 agent。没做。
- Android 的大文件上传要不要前台服务。小东西(几乎全部分享)根本碰不到。
- 分享单里要不要让人选发给哪个 agent(配了多个时)。现在一律发给当前挂着的那个 —— 扩展里根本读不到 roster,因为它不需要。
- 微信这类 app 分享出来的是 `content://` 临时授权,权限窗口有多长没量过。

_唤起机制:**四种写法里三种是死的,其中一种直接崩**(`UIApplication.shared.open` 编译不过;`NSExtensionContext.open` 报 `success = false`;responder chain 上的 `openURL:` 从 iOS 18 起被强制返回 NO;它的非废弃版在 UIKit 里崩)。能用的是 SwiftUI 的 `EnvironmentValues().openURL` 指向一条 Universal Link —— 这也是为什么扩展有一屏一个按钮而不是零界面:那一下点击是可靠触发的形状。还有一个只有真机能发现的顺序:`completeRequest` 拆界面会把还没注册完的 open 静默取消,所以中间要隔一个 runloop tick(50 ms,抄自一个跑通了的实现)。_

_机制:两扇门,按东西的形状分 —— 文件走 `POST /api/in/file`(multipart,不带 `note`),链接和文字走 `POST /api/in/text`。两边都**没有大小上限**:core 把 multipart 的每个 field 边收边写进 blob(`files::ingest_field`),客户端也从不把内容读进内存。_

_iOS 的形状是**扩展只排队,app 才发送**([`HandedDrop`](../../app/apple/ios/Shared/HandedDrop.swift))。原因不是审美:分享扩展的 sheet 一关进程就被杀,而且它要发就得共享 keychain。改成排队之后,唯一新增的 entitlement 就是一个 App Group,没有 keychain 共享、没有 roster 迁移、没有后台 URLSession,队列本身还成了重试缓冲 —— 连 [36](36-show-your-screen-from-a-button.md) 的截图也搬了上去,它以前失败一次就随进程丢了。Android 不需要这一整套:`ACTION_SEND` 直接启动 activity,同一个进程,东西到手时 app 已经开着。_

_状态:**一次都没跑过。** iOS `xcodebuild` 过(扩展已嵌入并通过 embedded-binary 校验),Android `make android` 过(编译 + 单测),core 那半 `make test` 过 —— 但**没有人打开过一次分享单**:没有 drop 被排过队,没有排过的 drop 到过 core,那条 Universal Link 没打开过任何东西。而且这条路**只有真机能验**:App Group 在 `CODE_SIGNING_ALLOWED=NO` 的构建里不工作(模拟器要按 [apple-ios.md](../platforms/apple-ios.md) ad-hoc 签名),Universal Link 还要从线上域名取到 AASA、对上真的 App ID。_
