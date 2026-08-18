import { url } from "../lib/base";
import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  type ReactNode,
  type RefObject,
} from "react";
import {
  MessageScroller,
  MessageScrollerButton,
  MessageScrollerContent,
  MessageScrollerItem,
  MessageScrollerProvider,
  MessageScrollerViewport,
  useMessageScroller,
} from "./shadcn/message-scroller";
import { Message, MessageContent, MessageGroup } from "./shadcn/message";
import { Bubble, BubbleContent } from "./shadcn/bubble";
import type { Message as ChatMessage } from "../channels/out/text";
import { splitSpeechLinks } from "../lib/links";
import { SenderAvatar } from "./Avatar";

/**
 * The conversation, as a chat between people.
 *
 * Deliberately not an agent transcript. There is no "Assistant" label, no copy /
 * regenerate / rating row, no reasoning trace, and no token-by-token fill — a
 * message appears when it is finished, because that is when a person sends one.
 * What is here instead is what a messenger has: consecutive messages from the same
 * sender grouped tight under one time, day separators, files shown as the thing that
 * was sent, and **a face beside each group**.
 *
 * The face is not decoration. This agent hears a room, so the person's side of the
 * conversation is not one person: a voiceprint places one line with 赵力 and leaves
 * the next unattributed. Until now the only sign of that was a `⟨voice: 赵力⟩` tag
 * rendered mid-sentence inside the bubble — evidence written for the mind, leaking
 * into the chat. The recognition is a field on the message now, drawn as an avatar
 * (`ui/Avatar.tsx`) and stripped out of the words.
 *
 * **There is no read receipt, and there will not be.** The scroller's own notion
 * of "you have not scrolled to this yet" stays in this component and is never
 * reported anywhere — see `docs/arch/host.md#attachment` for why that direction
 * is closed.
 *
 * The line the person writes on stands in its foot (`children`, which is
 * `ui/Composer.tsx`) rather than beside it. It is one surface: the messages and
 * the line that adds to them open, move and go away together, which is what lets
 * one control own the whole of it — see `docs/arch/stage.md`.
 *
 * **It is a card in both of its placements** — a title line, the messages, the
 * line being written — and the compositor decides only which box the card stands
 * in: the middle of the room, or the popover's corner over a view. The card was
 * previously the popover's own surface, so the same conversation was a bounded
 * panel over a view and a full-bleed column on the stage.
 */

/** A run of consecutive messages from one sender, rendered as one cluster. */
interface Group {
  key: string;
  role: ChatMessage["role"];
  /** Whoever the boundary said sent them; absent on the agent's own. */
  sender: ChatMessage["sender"];
  messages: ChatMessage[];
  /** Rendered above the group when the day changes. */
  daySeparator?: string;
}

const DAY = new Intl.DateTimeFormat(undefined, {
  weekday: "long",
  month: "short",
  day: "numeric",
});
const TIME = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });

function dayKey(ts: string): string {
  return ts.slice(0, 10);
}

function daySeparator(ts: string): string {
  const then = new Date(ts);
  const today = new Date();
  const yesterday = new Date(today);
  yesterday.setDate(today.getDate() - 1);
  if (dayKey(then.toISOString()) === dayKey(today.toISOString())) return "Today";
  if (dayKey(then.toISOString()) === dayKey(yesterday.toISOString())) return "Yesterday";
  return DAY.format(then);
}

/**
 * Cluster consecutive same-sender messages, and mark where the day turns over.
 *
 * A gap in time also breaks a group: three lines sent in one breath read as one
 * thought, while the same three spread over an afternoon do not, and stacking
 * them tight would say they arrived together.
 */
const GROUP_GAP_MS = 4 * 60 * 1000;

/**
 * Whether two messages came from the same person. **Role is not enough**: everyone
 * in the room is `user`, so grouping on it alone stacks 赵力's line and a colleague's
 * reply under one avatar and says the wrong person said both.
 *
 * Two unattributed lines do group together. We cannot tell whether they were the
 * same voice, and one silhouette over both claims no more than that.
 */
function sameSender(a: ChatMessage, b: ChatMessage): boolean {
  return a.sender?.subject === b.sender?.subject;
}

export function groupMessages(messages: ChatMessage[]): Group[] {
  const groups: Group[] = [];
  let previousDay: string | null = null;

  for (const message of messages) {
    const day = dayKey(message.ts);
    const separator = day !== previousDay ? daySeparator(message.ts) : undefined;
    previousDay = day;

    const open = groups[groups.length - 1];
    const last = open?.messages[open.messages.length - 1];
    const continues =
      open !== undefined &&
      last !== undefined &&
      separator === undefined &&
      open.role === message.role &&
      sameSender(last, message) &&
      new Date(message.ts).getTime() - new Date(last.ts).getTime() < GROUP_GAP_MS;

    if (continues && open) {
      open.messages.push(message);
      continue;
    }
    groups.push({
      key: message.id,
      role: message.role,
      sender: message.sender,
      messages: [message],
      ...(separator ? { daySeparator: separator } : {}),
    });
  }
  return groups;
}

function Body({ text }: { text: string }) {
  return (
    <>
      {splitSpeechLinks(text).map((part, index) =>
        part.kind === "link" ? (
          <a
            key={`${part.href}-${index}`}
            className="hi-speech-link"
            href={part.href}
            target="_blank"
            rel="noopener noreferrer"
            title={part.href}
          >
            <span className="hi-speech-link-label">{part.label}</span>
            <span aria-hidden>↗</span>
          </a>
        ) : (
          part.text
        ),
      )}
    </>
  );
}

/** A file the person handed over, shown as the thing they sent. */
function AttachmentView({ attachment }: { attachment: NonNullable<ChatMessage["attachment"]> }) {
  const src = url(`/api/media/${attachment.ref}`);
  if (attachment.mime.startsWith("image/")) {
    return (
      <img
        src={src}
        alt=""
        className="max-h-80 w-auto max-w-full rounded-[10px] object-contain"
        loading="lazy"
      />
    );
  }
  if (attachment.mime.startsWith("video/")) {
    return <video src={src} controls className="max-h-80 w-auto max-w-full rounded-[10px]" />;
  }
  return (
    <a href={src} target="_blank" rel="noopener noreferrer" className="underline underline-offset-2">
      Open file
    </a>
  );
}

export interface ChatProps {
  messages: ChatMessage[];
  /** The conversation's foot — the line being written. Rendered here so it is
   * part of the panel rather than a second box held flush with it by hand. */
  children?: ReactNode;
  /** The live recognition partial, shown pending at the tail. */
  interim?: string | undefined;
  /** Prepend a page of older messages; resolves to how many arrived. */
  onLoadOlder?: () => Promise<number>;
}

export function Chat({ messages, interim, onLoadOlder, children }: ChatProps) {
  const groups = useMemo(() => groupMessages(messages), [messages]);
  const viewportRef = useRef<HTMLDivElement | null>(null);
  // What is at the foot right now: the newest message, and the partial being
  // recognized under it. Either one changing means the tail moved.
  const tail = `${messages[messages.length - 1]?.id ?? ""}|${interim ?? ""}`;
  return (
    <MessageScrollerProvider autoScroll defaultScrollPosition="end">
      <div className="hi-chat">
        {/* One line, and it names who is on the other side of the conversation.
            Nothing else belongs in it: there is no thread to title (the record is
            one append-only list — `docs/arch/text-transcript.md`), so no "new
            chat" to offer beside it, and the agent's current state is the status
            button's job in the controls cluster. */}
        <header className="hi-chat-head">
          <img src={url("/icon.svg")} alt="" className="hi-chat-mark" />
          <span className="hi-chat-title">Hi Agent</span>
        </header>
        <MessageScroller className="hi-chat-scroller">
          <ScrollbackTrigger
            onLoadOlder={onLoadOlder}
            oldestId={messages[0]?.id}
            viewportRef={viewportRef}
          />
          <StickToBottom tail={tail} viewportRef={viewportRef} />
          <MessageScrollerViewport ref={viewportRef} preserveScrollOnPrepend className="px-4 py-6">
            <MessageScrollerContent className="mx-auto w-full max-w-[52rem] gap-6">
              {/* One group is one item, and the item is a DIRECT child of the content.
                  The scroller reads `data-message-id` off its own children only, so
                  the wrapper div that used to hold the day separator hid every message
                  from it — which quietly made `preserveScrollOnPrepend` a no-op, and
                  scrolling back landed at the top of the page just fetched instead of
                  holding the line being read. The separator goes inside the item.

                  No `scrollAnchor`: that pins the newest item to the TOP of the
                  viewport, which is the shape for a reply unfolding under your
                  question. A messenger follows its foot instead — see `StickToBottom`. */}
              {groups.map((group) => (
                <MessageScrollerItem key={group.key} messageId={group.key}>
                  {group.daySeparator && (
                    <div className="my-4 text-center text-xs text-muted-foreground">
                      {group.daySeparator}
                    </div>
                  )}
                  <Message align={group.role === "user" ? "end" : "start"}>
                    {/* First child, so `Message`'s own row-reverse on the person's
                        side puts it on the outer edge without a second rule. */}
                    <SenderAvatar sender={group.sender} role={group.role} />
                    <MessageContent>
                      {/* `w-full` is load-bearing, and it is what makes the bubbles'
                          `max-w-[80%]` mean 80% OF THE RAIL. `MessageContent` puts
                          `self-end` on its children on the person's side, which takes
                          the group off `stretch` and sizes it shrink-to-fit — i.e. to
                          the max-content of its widest bubble. Then that bubble's 80%
                          cap resolves against its own width and clips it: a line that
                          had room to spare wrapped one character early, every time,
                          because it was the longest in its run. (Capping the group at
                          `max-w-full` fixed the opposite overflow — one pasted
                          `file:///…` path dragging the whole run past the frame — but
                          left the percentage measuring the wrong box.) Full width
                          fixes both: the cap is honest, the bubbles right-align on
                          their own `self-end`, and long paths wrap inside their
                          bubble. */}
                      <MessageGroup className="w-full">
                        {group.messages.map((message) => (
                          <Bubble
                            key={message.id}
                            align={group.role === "user" ? "end" : "start"}
                            variant={group.role === "user" ? "secondary" : "default"}
                          >
                            <BubbleContent>
                              {message.attachment && (
                                <AttachmentView attachment={message.attachment} />
                              )}
                              {message.text && <Body text={message.text} />}
                            </BubbleContent>
                          </Bubble>
                        ))}
                      </MessageGroup>
                      <time
                        className="mt-1 block text-[11px] text-muted-foreground"
                        dateTime={group.messages[group.messages.length - 1]?.ts}
                      >
                        {TIME.format(new Date(group.messages[group.messages.length - 1]!.ts))}
                      </time>
                    </MessageContent>
                  </Message>
                </MessageScrollerItem>
              ))}

              {/* The line being recognized: a preview, so it sits outside the list
                  and is replaced by the real message when it settles. */}
              {interim && (
                <MessageScrollerItem>
                  <Message align="end">
                    {/* The avatar's space, held empty. Recognition has not settled, so
                        there is nobody to draw yet — and a silhouette that becomes a
                        face a second later would be the flicker, not the answer. The
                        spacer keeps the bubble on the same line it will land on. */}
                    <div className="size-7 shrink-0" aria-hidden />
                    <MessageContent>
                      <Bubble align="end" variant="outline" className="opacity-70">
                        <BubbleContent>{interim}</BubbleContent>
                      </Bubble>
                    </MessageContent>
                  </Message>
                </MessageScrollerItem>
              )}
            </MessageScrollerContent>
          </MessageScrollerViewport>
          <MessageScrollerButton direction="end" />
        </MessageScroller>
        {children && <div className="hi-chat-foot">{children}</div>}
      </div>
    </MessageScrollerProvider>
  );
}

/**
 * How far from the foot still counts as being at it. A hair more than the
 * scroller's own edge threshold, because a fractional device pixel or a momentum
 * bounce landing three pixels short must not read as "they scrolled away to read".
 */
const AT_FOOT_SLACK = 24;

/**
 * Keep the conversation on its newest message while the reader is at the foot of it.
 *
 * The scroller stops following the foot the moment it sees a wheel, a touch drag
 * or an arrow key — any of those means "I am reading, hold still" — and starts
 * following again only when a *scroll event* lands back at the foot. A gesture
 * that scrolls nothing emits no such event, so one trackpad flick at the bottom
 * of the list, where there was nowhere further to go, turned following off for
 * good. Every message after it appended below the fold behind an easily-missed
 * jump button, and the conversation looked like it had stopped — which is how a
 * `say` the backend holds, journalled and served, goes missing from the chat.
 *
 * So position decides, not gesture: while the last scroll left us at the foot, a
 * new message brings the foot back into view.
 */
function StickToBottom({
  tail,
  viewportRef,
}: {
  /** Changes whenever the foot of the list does. */
  tail: string;
  viewportRef: RefObject<HTMLDivElement | null>;
}) {
  const { scrollToEnd } = useMessageScroller();
  const atFootRef = useRef(true);

  useEffect(() => {
    const viewport = viewportRef.current;
    if (!viewport) return;
    const read = () => {
      atFootRef.current =
        viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight <= AT_FOOT_SLACK;
    };
    read();
    viewport.addEventListener("scroll", read, { passive: true });
    return () => viewport.removeEventListener("scroll", read);
  }, [viewportRef]);

  // Before paint, so a message never shows up half a screen down and then jumps.
  useLayoutEffect(() => {
    if (atFootRef.current) scrollToEnd({ behavior: "auto" });
  }, [tail, scrollToEnd]);

  return null;
}

/**
 * Ask for older messages when the viewport is scrolled near the top.
 *
 * Lives inside the provider because that is where the scroll state is, and it
 * renders nothing: `preserveScrollOnPrepend` on the viewport is what keeps the
 * page from jumping once they arrive.
 */
function ScrollbackTrigger({
  onLoadOlder,
  oldestId,
  viewportRef,
}: {
  onLoadOlder?: (() => Promise<number>) | undefined;
  oldestId?: string | undefined;
  viewportRef: RefObject<HTMLDivElement | null>;
}) {
  const exhaustedRef = useRef(false);
  // A different oldest message means the conversation grew backwards, so there
  // may be more behind it — reopen the door we closed.
  useEffect(() => {
    exhaustedRef.current = false;
  }, [oldestId]);

  useEffect(() => {
    if (!onLoadOlder) return;
    // This conversation's own viewport, held by ref rather than found in the
    // document: an agent view on the stage may have a scroller of its own, and a
    // document-wide query would happily drive that one instead.
    const viewport = viewportRef.current;
    if (!viewport) return;
    const onScroll = () => {
      if (exhaustedRef.current || viewport.scrollTop > 200) return;
      void onLoadOlder().then((added) => {
        if (added === 0) exhaustedRef.current = true;
      });
    };
    viewport.addEventListener("scroll", onScroll, { passive: true });
    return () => viewport.removeEventListener("scroll", onScroll);
  }, [onLoadOlder, viewportRef]);

  return null;
}
