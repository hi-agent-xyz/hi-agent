import { useEffect, useMemo, useRef } from "react";
import {
  MessageScroller,
  MessageScrollerButton,
  MessageScrollerContent,
  MessageScrollerItem,
  MessageScrollerProvider,
  MessageScrollerViewport,
} from "./shadcn/message-scroller";
import { Message, MessageContent, MessageGroup } from "./shadcn/message";
import { Bubble, BubbleContent } from "./shadcn/bubble";
import type { Message as ChatMessage } from "../channels/out/text";
import { splitSpeechLinks } from "../lib/links";

/**
 * The conversation, as a chat between two people.
 *
 * Deliberately not an agent transcript. There is no avatar, no "Assistant"
 * label, no copy / regenerate / rating row, no reasoning trace, and no
 * token-by-token fill — a message appears when it is finished, because that is
 * when a person sends one. What is here instead is what a messenger has:
 * consecutive messages from the same sender grouped tight under one time, day
 * separators, and files shown as the thing that was sent.
 *
 * **There is no read receipt, and there will not be.** The scroller's own notion
 * of "you have not scrolled to this yet" stays in this component and is never
 * reported anywhere — see `docs/arch/core.md#attachment` for why that direction
 * is closed.
 */

/** A run of consecutive messages from one sender, rendered as one cluster. */
interface Group {
  key: string;
  role: ChatMessage["role"];
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
      new Date(message.ts).getTime() - new Date(last.ts).getTime() < GROUP_GAP_MS;

    if (continues && open) {
      open.messages.push(message);
      continue;
    }
    groups.push({
      key: message.id,
      role: message.role,
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
  const src = `/api/media/${attachment.ref}`;
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
  /** The live recognition partial, shown pending at the tail. */
  interim?: string | undefined;
  /** Prepend a page of older messages; resolves to how many arrived. */
  onLoadOlder?: () => Promise<number>;
}

export function Chat({ messages, interim, onLoadOlder }: ChatProps) {
  const groups = useMemo(() => groupMessages(messages), [messages]);
  return (
    <MessageScrollerProvider autoScroll defaultScrollPosition="end">
      <MessageScroller className="hi-chat">
        <ScrollbackTrigger onLoadOlder={onLoadOlder} oldestId={messages[0]?.id} />
        <MessageScrollerViewport preserveScrollOnPrepend className="px-4 py-6">
          <MessageScrollerContent className="mx-auto w-full max-w-[52rem] gap-6">
            {groups.map((group) => (
              <div key={group.key}>
                {group.daySeparator && (
                  <div className="my-4 text-center text-xs text-muted-foreground">
                    {group.daySeparator}
                  </div>
                )}
                <MessageScrollerItem messageId={group.key} scrollAnchor>
                  <Message align={group.role === "user" ? "end" : "start"}>
                    <MessageContent>
                      <MessageGroup>
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
              </div>
            ))}

            {/* The line being recognized: a preview, so it sits outside the list
                and is replaced by the real message when it settles. */}
            {interim && (
              <MessageScrollerItem>
                <Message align="end">
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
    </MessageScrollerProvider>
  );
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
}: {
  onLoadOlder?: (() => Promise<number>) | undefined;
  oldestId?: string | undefined;
}) {
  const exhaustedRef = useRef(false);
  // A different oldest message means the conversation grew backwards, so there
  // may be more behind it — reopen the door we closed.
  useEffect(() => {
    exhaustedRef.current = false;
  }, [oldestId]);

  useEffect(() => {
    if (!onLoadOlder) return;
    const viewport = document.querySelector<HTMLElement>('[data-slot="message-scroller-viewport"]');
    if (!viewport) return;
    const onScroll = () => {
      if (exhaustedRef.current || viewport.scrollTop > 200) return;
      void onLoadOlder().then((added) => {
        if (added === 0) exhaustedRef.current = true;
      });
    };
    viewport.addEventListener("scroll", onScroll, { passive: true });
    return () => viewport.removeEventListener("scroll", onScroll);
  }, [onLoadOlder]);

  return null;
}
