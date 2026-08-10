// purpose: 任务 — the task ledger as one board: todo / doing / done / cancelled side by side.
// The full canvas is organized by the one durable lifecycle: todo, doing, done,
// cancelled. Liveness is optional detail on a doing task, never a second kind or mode.
//
// The board is the whole page — four columns, each scrolling on its own, nothing
// hidden behind a filter. A card is therefore a *glance*: a clamped title, the one or
// two facts that decide whether it needs you, and the actions. Everything long —
// the untruncated title, the prose, the liveness contract — lives in the detail panel
// a card opens. Titles arrive as whatever the agent wrote, and some are paragraphs;
// the card truncates rather than letting one task eat the column.
//
// A status change is a card moved between columns, and dragging one there is the
// gesture the layout already promises. It is never the *only* way: HTML5 drag does not
// exist on touch and cannot be driven from a keyboard, so every card keeps its buttons
// and the drag is the shortcut on top of them.
import { useState, useEffect, useCallback, useRef } from "react";

const J = { "Content-Type": "application/json" };
const api = {
  list: () => fetch("/api/tasks").then((response) => response.json()),
  patch: (subject, patch) =>
    fetch(`/api/tasks/${encodeURIComponent(subject)}`, {
      method: "PATCH",
      headers: J,
      body: JSON.stringify(patch),
    }).then((response) => response.json()),
};

const T = {
  en: {
    title: "Tasks",
    activeN: (n) => `${n} active`,
    totalN: (n) => `${n} ${n === 1 ? "task" : "tasks"}`,
    board: "Task board",
    category: {
      todo: "Todo",
      doing: "Doing",
      done: "Done",
      cancelled: "Cancelled",
    },
    empty: {
      todo: "Nothing queued.",
      doing: "Nothing in progress.",
      done: "Nothing finished yet.",
      cancelled: "Nothing cancelled.",
    },
    attentionN: (n) => `${n} need attention`,
    created: (at) => `Created ${at}`,
    completed: (at) => `Completed ${at}`,
    cancelled: (at) => `Cancelled ${at}`,
    due: (at) => `Due ${at}`,
    overdue: (at) => `Overdue ${at}`,
    checked: (at) => `Checked ${at}`,
    neverChecked: "Never checked",
    start: "Start",
    moveTodo: "Move to todo",
    markDone: "Mark done",
    cancel: "Cancel",
    reopen: "Reopen",
    // Short forms for the card, where a column is a few hundred pixels wide. The
    // full wording above stays as the button's label for anyone not reading pixels.
    shortStart: "Start",
    shortTodo: "Todo",
    shortDone: "Done",
    shortCancel: "Cancel",
    shortReopen: "Reopen",
    details: "Open details",
    close: "Close",
    malformed: "This task has invalid stored fields. Changing its status will rewrite the recognized fields.",
    malformedShort: "Invalid fields",
    noBody: "(no notes)",
    monitoring: "Liveness",
    verify: "Check",
    restart: "If it stops",
    owner: "Owner",
    startKey: "Start key",
  },
  zh: {
    title: "任务",
    activeN: (n) => `${n} 件进行中`,
    totalN: (n) => `${n} 件任务`,
    board: "任务看板",
    category: {
      todo: "待办",
      doing: "进行中",
      done: "已完成",
      cancelled: "已取消",
    },
    empty: {
      todo: "没有待办。",
      doing: "没有进行中的。",
      done: "还没有完成的。",
      cancelled: "没有已取消的。",
    },
    attentionN: (n) => `${n} 件需要留意`,
    created: (at) => `创建于 ${at}`,
    completed: (at) => `完成于 ${at}`,
    cancelled: (at) => `取消于 ${at}`,
    due: (at) => `截止 ${at}`,
    overdue: (at) => `已逾期 ${at}`,
    checked: (at) => `检查于 ${at}`,
    neverChecked: "尚未检查",
    start: "开始",
    moveTodo: "移回待办",
    markDone: "完成",
    cancel: "取消",
    reopen: "重新打开",
    shortStart: "开始",
    shortTodo: "待办",
    shortDone: "完成",
    shortCancel: "取消",
    shortReopen: "重开",
    details: "查看详情",
    close: "关闭",
    malformed: "这条任务包含无效字段。修改状态时会重写可识别的字段。",
    malformedShort: "字段无效",
    noBody: "（没有备注）",
    monitoring: "运行检查",
    verify: "检查方式",
    restart: "停止后",
    owner: "负责人",
    startKey: "启动标识",
  },
};

// Resolves to both the table and the tag that chose it: the dates on this surface are
// formatted by `Intl`, which needs a locale, and the honest one is the language the copy
// around it is already in. `undefined` there reads the *system* locale instead, so a zh
// reader on an en machine got "创建于 Aug 8, 3:04 PM".
function words() {
  const app = document.documentElement.lang || "";
  const chain = !app || /^system$/i.test(app) ? [navigator.language] : [app, navigator.language];
  for (const tag of chain) {
    if (/^zh\b/i.test(tag || "")) return [T.zh, tag];
    if (/^en\b/i.test(tag || "")) return [T.en, tag];
  }
  return [T.en, "en"];
}
const [L, LOCALE] = words();

const STATUSES = [
  { id: "todo", label: L.category.todo, tone: "mute" },
  { id: "doing", label: L.category.doing, tone: "accent" },
  { id: "done", label: L.category.done, tone: "secondary" },
  { id: "cancelled", label: L.category.cancelled, tone: "danger" },
];

const STATUS_TONE = {
  todo: "var(--fg-dim)",
  doing: "var(--accent)",
  done: "var(--accent-2)",
  cancelled: "var(--danger)",
};

export default function Tasks() {
  const [tasks, setTasks] = useState(null);
  const [busy, setBusy] = useState(null);
  // The open detail panel is held by subject, not by task object: the poll below
  // replaces every task on each tick, and a held object would freeze the panel on
  // the version that was open when it was clicked.
  const [openSubject, setOpenSubject] = useState(null);
  // The card in hand: `{ subject, status }`, so a column can tell whether a drop over it
  // would change anything before it offers to accept one.
  const [drag, setDrag] = useState(null);
  // Refs, not the state: the poll interval is created once and would otherwise close
  // over the `busy` and `drag` of its first render.
  const busyRef = useRef(false);
  const dragRef = useRef(false);

  const reload = useCallback(async () => {
    const data = await api.list().catch(() => ({ tasks: [] }));
    setTasks(data.tasks || []);
  }, []);

  // Poll, for the reason the workers roster does: the agent opens and closes tasks while
  // this is on screen, and a ledger that is quietly stale still reads as authoritative —
  // it is the surface someone checks *before* asking "did you drop that?". Held off while
  // a status write is in flight (so a card can't flip back under the click), while a card
  // is mid-drag (re-rendering the board out from under a held card cancels the drag), and
  // while the page is hidden, since nothing is being read then.
  useEffect(() => {
    reload();
    const timer = setInterval(() => {
      if (!document.hidden && !busyRef.current && !dragRef.current) reload();
    }, 8000);
    return () => clearInterval(timer);
  }, [reload]);

  const setTaskStatus = async (subject, nextStatus) => {
    setBusy(subject);
    busyRef.current = true;
    await api.patch(subject, { status: nextStatus }).catch(() => {});
    setBusy(null);
    busyRef.current = false;
    reload();
  };

  const startDrag = (task) => {
    dragRef.current = true;
    setDrag({ subject: task.subject, status: task.status });
  };

  const endDrag = () => {
    dragRef.current = false;
    setDrag(null);
  };

  // A drop onto the column a card came from is not a change — dropping there is how you
  // cancel a drag, so it must not spend a write or flash the card busy.
  const dropOn = (subject, nextStatus) => {
    endDrag();
    const task = tasks?.find((item) => item.subject === subject);
    if (!task || task.status === nextStatus) return;
    setTaskStatus(subject, nextStatus);
  };

  if (tasks === null) {
    return (
      <div className="hi-tasks">
        <style>{CSS}</style>
        <Header activeCount={0} totalCount={0} />
        <div className="hi-tasks__loading" aria-label={L.title}>
          <span />
          <span />
          <span />
          <span />
        </div>
      </div>
    );
  }

  const activeCount = tasks.filter((task) => task.status === "todo" || task.status === "doing").length;
  // A task whose status changed out from under the panel keeps the panel open on it —
  // it is still the task someone was reading. Only a task that left the ledger closes it.
  const open = openSubject ? tasks.find((task) => task.subject === openSubject) || null : null;

  return (
    <div className="hi-tasks">
      <style>{CSS}</style>
      <Header activeCount={activeCount} totalCount={tasks.length} />

      <div className="hi-tasks__board" aria-label={L.board}>
        {STATUSES.map((column) => (
          <Column
            key={column.id}
            column={column}
            tasks={tasks.filter((task) => task.status === column.id)}
            busy={busy}
            drag={drag}
            onStatus={setTaskStatus}
            onOpen={setOpenSubject}
            onDragStart={startDrag}
            onDragEnd={endDrag}
            onDrop={dropOn}
          />
        ))}
      </div>

      {open && (
        <Detail
          task={open}
          busy={busy === open.subject}
          onStatus={setTaskStatus}
          onClose={() => setOpenSubject(null)}
        />
      )}
    </div>
  );
}

function Header({ activeCount, totalCount }) {
  return (
    <header className="hi-tasks__header">
      <div className="hi-tasks__heading">
        <h1>{L.title}</h1>
        <span>{L.activeN(activeCount)}</span>
      </div>
      <div className="hi-tasks__total">{L.totalN(totalCount)}</div>
    </header>
  );
}

function Column({ column, tasks, busy, drag, onStatus, onOpen, onDragStart, onDragEnd, onDrop }) {
  const attention = tasks.filter(taskNeedsAttention).length;
  const [over, setOver] = useState(false);
  // Only a card from another column can land here. Without this the source column also
  // lights up as a target, which reads as "this does something" when it does not.
  const takes = Boolean(drag) && drag.status !== column.id;

  return (
    <section
      className="hi-tasks__column"
      data-tone={column.tone}
      data-drop={takes && over ? "true" : undefined}
      aria-label={`${column.label} (${tasks.length})`}
      onDragOver={(event) => {
        if (!takes) return;
        // Preventing the default is what makes an element a drop target at all.
        event.preventDefault();
        event.dataTransfer.dropEffect = "move";
        setOver(true);
      }}
      onDragLeave={(event) => {
        // `dragleave` also fires crossing into a child, so only a departure that leaves
        // the column entirely counts.
        if (!event.currentTarget.contains(event.relatedTarget)) setOver(false);
      }}
      onDrop={(event) => {
        if (!takes) return;
        event.preventDefault();
        setOver(false);
        onDrop(event.dataTransfer.getData("text/plain") || drag.subject, column.id);
      }}
    >
      <div className="hi-tasks__column-head">
        <span className="hi-tasks__column-dot" aria-hidden="true" />
        <h2 className="hi-tasks__column-label">{column.label}</h2>
        <span className="hi-tasks__column-count">{tasks.length}</span>
        {attention > 0 && (
          <span className="hi-tasks__column-attention" title={L.attentionN(attention)}>
            {attention}
          </span>
        )}
      </div>

      {tasks.length === 0 ? (
        <div className="hi-tasks__empty">{L.empty[column.id]}</div>
      ) : (
        <div className="hi-tasks__cards">
          {tasks.map((task) => (
            <Card
              key={task.subject}
              task={task}
              busy={busy === task.subject}
              dragging={drag?.subject === task.subject}
              onStatus={onStatus}
              onOpen={onOpen}
              onDragStart={onDragStart}
              onDragEnd={onDragEnd}
            />
          ))}
        </div>
      )}
    </section>
  );
}

function Card({ task, busy, dragging, onStatus, onOpen, onDragStart, onDragEnd }) {
  const notes = cardNotes(task);
  // A drag that ends on the card it started from still delivers a `click`, which would
  // open the panel on a gesture the person meant as "put it back".
  const dragged = useRef(false);

  return (
    <article
      className="hi-tasks__card"
      draggable
      data-malformed={task.malformed ? "true" : undefined}
      data-dragging={dragging ? "true" : undefined}
      aria-busy={busy}
      style={{ "--task-status": STATUS_TONE[task.status] || "var(--fg-mute)" }}
      onDragStart={(event) => {
        dragged.current = true;
        // Firefox refuses to start a drag with an empty payload; the subject is also
        // what the drop handler reads back, so this is the payload, not a placebo.
        event.dataTransfer.setData("text/plain", task.subject);
        event.dataTransfer.effectAllowed = "move";
        onDragStart(task);
      }}
      onDragEnd={() => {
        onDragEnd();
        // Cleared after the click that may follow this drag has been swallowed.
        setTimeout(() => {
          dragged.current = false;
        }, 0);
      }}
    >
      <button
        type="button"
        className="hi-tasks__card-open"
        // The card clamps to two lines, so the untruncated title has to be reachable
        // without opening the panel — this is the hover that gives it.
        title={`${task.title || task.subject}\n\n${L.details}`}
        onClick={() => {
          if (dragged.current) return;
          onOpen(task.subject);
        }}
      >
        <span className="hi-tasks__card-title">{task.title || task.subject}</span>
        {notes.length > 0 && (
          <span className="hi-tasks__card-notes">
            {notes.map((note) => (
              <span key={note.text} data-warn={note.warn ? "true" : undefined}>
                {note.text}
              </span>
            ))}
          </span>
        )}
      </button>

      <Actions task={task} busy={busy} onStatus={onStatus} short />
    </article>
  );
}

function Detail({ task, busy, onStatus, onClose }) {
  const panel = useRef(null);

  useEffect(() => {
    panel.current?.focus();
    const onKey = (event) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  const due = dueMeta(task);
  const health = healthMeta(task);
  const closedAt =
    task.status === "done" && task.completedAt
      ? L.completed(formatStamp(task.completedAt))
      : task.status === "cancelled" && task.cancelledAt
        ? L.cancelled(formatStamp(task.cancelledAt))
        : null;

  return (
    <div className="hi-tasks__scrim" onClick={onClose}>
      <div
        ref={panel}
        className="hi-tasks__detail"
        role="dialog"
        aria-modal="true"
        aria-label={task.title || task.subject}
        tabIndex={-1}
        style={{ "--task-status": STATUS_TONE[task.status] || "var(--fg-mute)" }}
        onClick={(event) => event.stopPropagation()}
      >
        <div className="hi-tasks__detail-head">
          <span className="hi-tasks__chip">{L.category[task.status] || task.status}</span>
          <button type="button" className="hi-tasks__close" aria-label={L.close} onClick={onClose}>
            ×
          </button>
        </div>

        <div className="hi-tasks__detail-scroll">
          <h3 className="hi-tasks__detail-title">{task.title || task.subject}</h3>

          <div className="hi-tasks__detail-meta">
            {task.createdAt && <span>{L.created(formatStamp(task.createdAt))}</span>}
            {closedAt && <span>{closedAt}</span>}
            {due && <span data-warn={due.warn ? "true" : undefined}>{due.text}</span>}
            {health && <span data-warn={health.warn ? "true" : undefined}>{health.text}</span>}
          </div>

          {task.malformed && <div className="hi-tasks__bad">{L.malformed}</div>}

          {task.body ? (
            <div className="hi-tasks__prose">{task.body}</div>
          ) : (
            <div className="hi-tasks__none">{L.noBody}</div>
          )}

          {task.liveness && (
            <div className="hi-tasks__liveness">
              <div className="hi-tasks__liveness-title">{L.monitoring}</div>
              {task.liveness.verify && <div><b>{L.verify}:</b> {task.liveness.verify}</div>}
              {task.liveness.restart && <div><b>{L.restart}:</b> {task.liveness.restart}</div>}
              {task.liveness.owner && <div><b>{L.owner}:</b> {task.liveness.owner}</div>}
              {task.liveness.startKey && <div><b>{L.startKey}:</b> {task.liveness.startKey}</div>}
            </div>
          )}

          <div className="hi-tasks__subject">{task.subject}</div>
        </div>

        <Actions task={task} busy={busy} onStatus={onStatus} />
      </div>
    </div>
  );
}

// `short` swaps the card's cramped labels in while keeping the full wording as the
// accessible name — a column is too narrow for "Move to todo", a screen reader is not.
function Actions({ task, busy, onStatus, short }) {
  const button = (kind, status, label, shortLabel) => (
    <button
      type="button"
      className={`hi-tasks__button hi-tasks__button--${kind}`}
      disabled={busy}
      aria-label={label}
      title={label}
      onClick={() => onStatus(task.subject, status)}
    >
      {short ? shortLabel : label}
    </button>
  );

  // `draggable={false}` so pressing a button never starts a drag of the card around it:
  // a draggable element otherwise hands its whole subtree to the drag.
  if (task.status === "done" || task.status === "cancelled") {
    return (
      <div className="hi-tasks__actions" draggable={false}>
        {button("ghost", "todo", L.reopen, L.shortReopen)}
      </div>
    );
  }

  return (
    <div className="hi-tasks__actions" draggable={false}>
      {task.status === "todo"
        ? button("ghost", "doing", L.start, L.shortStart)
        : button("ghost", "todo", L.moveTodo, L.shortTodo)}
      {button("danger", "cancelled", L.cancel, L.shortCancel)}
      {button("primary", "done", L.markDone, L.shortDone)}
    </div>
  );
}

function formatStamp(value) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  const sameYear = date.getFullYear() === new Date().getFullYear();
  return new Intl.DateTimeFormat(LOCALE, {
    month: "short",
    day: "numeric",
    ...(sameYear ? {} : { year: "numeric" }),
    hour: "numeric",
    minute: "2-digit",
  }).format(date);
}

function dueMeta(task) {
  if (!task.dueAt) return null;
  const time = new Date(task.dueAt).getTime();
  if (Number.isNaN(time)) return null;
  const overdue =
    (task.status === "todo" || task.status === "doing") && time <= Date.now();
  return {
    text: overdue ? L.overdue(formatStamp(task.dueAt)) : L.due(formatStamp(task.dueAt)),
    warn: overdue,
  };
}

function healthMeta(task) {
  if (task.status !== "doing" || !task.liveness) return null;
  if (!task.checkedAt) return { text: L.neverChecked, warn: true };
  return { text: L.checked(formatStamp(task.checkedAt)), warn: false };
}

function taskNeedsAttention(task) {
  return Boolean(dueMeta(task)?.warn || healthMeta(task)?.warn);
}

// One clipped line under the title, so what earns the space is what would make someone
// act: a warning first, then the one date that means anything for this status.
function cardNotes(task) {
  const notes = [];
  if (task.malformed) notes.push({ text: L.malformedShort, warn: true });
  const due = dueMeta(task);
  const health = healthMeta(task);
  if (due?.warn) notes.push(due);
  if (health?.warn) notes.push(health);
  if (notes.length === 0) {
    if (due) notes.push(due);
    else if (health) notes.push(health);
  }
  if (task.status === "done" && task.completedAt) {
    notes.push({ text: L.completed(formatStamp(task.completedAt)) });
  } else if (task.status === "cancelled" && task.cancelledAt) {
    notes.push({ text: L.cancelled(formatStamp(task.cancelledAt)) });
  } else if (task.createdAt) {
    notes.push({ text: L.created(formatStamp(task.createdAt)) });
  }
  return notes;
}

const CSS = `
  .hi-tasks,
  .hi-tasks * {
    box-sizing: border-box;
  }

  .hi-tasks {
    position: relative;
    width: 100%;
    height: 100%;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: var(--bg-0);
    color: var(--fg);
    font-family: var(--font-display);
  }

  .hi-tasks button {
    cursor: pointer;
  }

  .hi-tasks button:focus-visible {
    outline: 3px solid var(--accent-soft);
    outline-offset: 2px;
  }

  .hi-tasks__header {
    flex: none;
    min-height: 64px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 20px;
    padding: 14px clamp(16px, 2.4vw, 30px);
    border-bottom: 1px solid var(--line);
  }

  .hi-tasks__heading {
    min-width: 0;
    display: flex;
    align-items: baseline;
    gap: 12px;
  }

  .hi-tasks__heading h1 {
    margin: 0;
    font-size: 24px;
    line-height: 1.1;
    font-weight: 800;
    letter-spacing: 0;
  }

  .hi-tasks__heading span,
  .hi-tasks__total {
    color: var(--fg-dim);
    font-size: 13px;
    font-weight: 650;
    font-variant-numeric: tabular-nums;
  }

  /* The board is the page: four columns that fill the height and never scroll it.
     Below ~1100px the columns keep a readable floor and the board scrolls sideways
     instead of squeezing every card into a ribbon. */
  .hi-tasks__board {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: repeat(4, minmax(250px, 1fr));
    gap: 12px;
    padding: 14px clamp(16px, 2.4vw, 30px) 16px;
    overflow-x: auto;
    overflow-y: hidden;
  }

  .hi-tasks__column {
    --column-tone: var(--fg-mute);
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border: 1px solid var(--line);
    border-radius: 10px;
    background: color-mix(in srgb, var(--surface) 42%, var(--bg-0));
  }

  .hi-tasks__column[data-tone="accent"] { --column-tone: var(--accent); }
  .hi-tasks__column[data-tone="secondary"] { --column-tone: var(--accent-2); }
  .hi-tasks__column[data-tone="danger"] { --column-tone: var(--danger); }

  /* Only the column a held card would actually land in lights up. */
  .hi-tasks__column[data-drop="true"] {
    border-color: var(--accent-line);
    background: var(--accent-wash);
    box-shadow: inset 0 0 0 1px var(--accent-line);
  }

  .hi-tasks__column-head {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 11px 12px;
    border-bottom: 1px solid var(--line);
  }

  .hi-tasks__column-dot {
    flex: none;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--column-tone);
  }

  .hi-tasks__column-label {
    min-width: 0;
    margin: 0;
    overflow: hidden;
    color: var(--fg);
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13.5px;
    line-height: 1.3;
    font-weight: 780;
  }

  .hi-tasks__column-count {
    margin-left: auto;
    min-width: 22px;
    height: 22px;
    display: grid;
    place-items: center;
    padding: 0 6px;
    border-radius: 999px;
    background: var(--accent-wash);
    color: var(--fg-dim);
    font-size: 11.5px;
    line-height: 1;
    font-weight: 750;
    font-variant-numeric: tabular-nums;
  }

  .hi-tasks__column-attention {
    min-width: 22px;
    height: 22px;
    display: grid;
    place-items: center;
    padding: 0 6px;
    border-radius: 999px;
    background: var(--danger-wash);
    color: var(--danger);
    font-size: 11.5px;
    line-height: 1;
    font-weight: 780;
    font-variant-numeric: tabular-nums;
  }

  .hi-tasks__cards {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px;
    overflow-y: auto;
  }

  /* The floating dock sits over the bottom-right corner of the surface, which is the
     last column's tail. Only that column pays for the clearance. */
  .hi-tasks__column:last-child .hi-tasks__cards {
    padding-bottom: 76px;
  }

  .hi-tasks__card {
    flex: none;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border: 1px solid var(--line);
    border-left: 3px solid var(--task-status);
    border-radius: 8px;
    background: var(--surface);
    transition: background 160ms var(--ease), border-color 160ms var(--ease);
  }

  .hi-tasks__card:hover {
    background: var(--surface-strong);
    border-top-color: var(--line-strong);
    border-right-color: var(--line-strong);
    border-bottom-color: var(--line-strong);
  }

  .hi-tasks__card[data-malformed="true"] {
    border-top-color: var(--danger-line);
    border-right-color: var(--danger-line);
    border-bottom-color: var(--danger-line);
  }

  /* The card left behind while its ghost is under the cursor. */
  .hi-tasks__card[data-dragging="true"] {
    opacity: 0.4;
  }

  .hi-tasks__card-open {
    width: 100%;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 11px 12px 8px;
    text-align: left;
  }

  /* Two lines, then it stops. A title is a name here — the paragraph some of them
     carry belongs to the detail panel, and the card must not grow to hold it. */
  .hi-tasks__card-title {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    color: var(--fg);
    font-size: 13.5px;
    line-height: 1.45;
    font-weight: 700;
    overflow-wrap: anywhere;
  }

  .hi-tasks__card-notes {
    display: flex;
    align-items: baseline;
    gap: 10px;
    overflow: hidden;
    color: var(--fg-dim);
    white-space: nowrap;
    font-size: 11.5px;
    line-height: 1.4;
    font-weight: 620;
    font-variant-numeric: tabular-nums;
  }

  .hi-tasks__card-notes span {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .hi-tasks__card-notes [data-warn="true"] {
    flex: none;
    color: var(--danger);
  }

  .hi-tasks__actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding: 0 10px 10px;
  }

  .hi-tasks__button {
    min-height: 30px;
    padding: 0 10px;
    border-radius: 6px;
    font-size: 12px;
    font-weight: 720;
    white-space: nowrap;
    transition: background 160ms var(--ease), border-color 160ms var(--ease), opacity 160ms var(--ease);
  }

  .hi-tasks__button:disabled {
    cursor: default;
    opacity: 0.45;
  }

  .hi-tasks__button--ghost,
  .hi-tasks__button--danger {
    border: 1px solid var(--line-strong);
    color: var(--fg-dim);
  }

  .hi-tasks__button--ghost:hover:not(:disabled) {
    border-color: var(--accent-line);
    background: var(--accent-wash);
    color: var(--fg);
  }

  .hi-tasks__button--danger:hover:not(:disabled) {
    border-color: var(--danger-line);
    background: var(--danger-wash);
    color: var(--danger);
  }

  .hi-tasks__button--primary {
    border: 1px solid color-mix(in srgb, var(--accent) 82%, var(--fg));
    background: color-mix(in srgb, var(--accent) 82%, var(--fg));
    color: var(--bg-0);
  }

  .hi-tasks__button--primary:hover:not(:disabled) {
    filter: brightness(0.94);
  }

  .hi-tasks__empty {
    flex: 1;
    display: grid;
    place-content: center;
    padding: 24px 16px;
    color: var(--fg-mute);
    text-align: center;
    font-size: 12.5px;
    line-height: 1.45;
    font-weight: 650;
  }

  /* Absolute, not fixed: the view is a surface inside the app, and a transformed
     ancestor would silently re-anchor a fixed overlay. */
  .hi-tasks__scrim {
    position: absolute;
    inset: 0;
    z-index: 10;
    display: grid;
    place-items: center;
    padding: clamp(16px, 4vh, 48px) clamp(16px, 4vw, 48px);
    background: color-mix(in srgb, var(--bg-0) 62%, transparent);
    backdrop-filter: blur(6px);
    -webkit-backdrop-filter: blur(6px);
  }

  .hi-tasks__detail {
    width: min(640px, 100%);
    max-height: 100%;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border: 1px solid var(--line-strong);
    border-radius: 12px;
    background: var(--bg-0);
    box-shadow: 0 24px 60px var(--shadow-strong);
  }

  .hi-tasks__detail:focus-visible {
    outline: none;
  }

  .hi-tasks__detail-head {
    flex: none;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 12px 12px 12px 16px;
    border-bottom: 1px solid var(--line);
  }

  .hi-tasks__chip {
    padding: 3px 8px;
    border-radius: 5px;
    background: color-mix(in srgb, var(--task-status) 12%, transparent);
    color: var(--task-status);
    font-size: 12px;
    font-weight: 750;
  }

  .hi-tasks__close {
    width: 30px;
    height: 30px;
    display: grid;
    place-items: center;
    border-radius: 6px;
    color: var(--fg-dim);
    font-size: 20px;
    line-height: 1;
  }

  .hi-tasks__close:hover {
    background: var(--surface-strong);
    color: var(--fg);
  }

  .hi-tasks__detail-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 16px;
  }

  .hi-tasks__detail-title {
    margin: 0 0 10px;
    color: var(--fg);
    font-size: 16px;
    line-height: 1.5;
    font-weight: 760;
    overflow-wrap: anywhere;
  }

  .hi-tasks__detail-meta {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px 14px;
    margin-bottom: 14px;
    color: var(--fg-dim);
    font-size: 12px;
    line-height: 1.35;
    font-weight: 620;
  }

  .hi-tasks__detail-meta [data-warn="true"] {
    color: var(--danger);
  }

  .hi-tasks__bad {
    margin-bottom: 14px;
    color: var(--danger);
    font-size: 12.5px;
    line-height: 1.5;
  }

  .hi-tasks__prose,
  .hi-tasks__none {
    color: var(--fg-dim);
    font-size: 13.5px;
    line-height: 1.65;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .hi-tasks__liveness {
    margin-top: 14px;
    padding: 12px 14px;
    border-left: 3px solid var(--accent-2);
    background: color-mix(in srgb, var(--accent-2) 7%, transparent);
    color: var(--fg-dim);
    font-size: 13px;
    line-height: 1.65;
  }

  .hi-tasks__liveness-title {
    margin-bottom: 5px;
    color: var(--fg-dim);
    font-size: 11.5px;
    font-weight: 750;
  }

  .hi-tasks__subject {
    margin-top: 12px;
    color: var(--fg-mute);
    font-size: 11.5px;
    line-height: 1.4;
    overflow-wrap: anywhere;
  }

  .hi-tasks__detail .hi-tasks__actions {
    flex: none;
    justify-content: flex-end;
    padding: 12px 16px;
    border-top: 1px solid var(--line);
  }

  .hi-tasks__detail .hi-tasks__button {
    min-height: 38px;
    padding: 0 13px;
    font-size: 13px;
    font-weight: 750;
  }

  .hi-tasks__loading {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: repeat(4, minmax(250px, 1fr));
    gap: 12px;
    padding: 14px clamp(16px, 2.4vw, 30px) 16px;
    overflow: hidden;
  }

  .hi-tasks__loading span {
    border: 1px solid var(--line);
    border-radius: 10px;
    background: color-mix(in srgb, var(--surface) 42%, var(--bg-0));
  }

  @media (max-width: 760px) {
    .hi-tasks__header {
      min-height: 56px;
      padding: 12px 16px;
    }

    .hi-tasks__heading {
      gap: 9px;
    }

    .hi-tasks__heading h1 {
      font-size: 21px;
    }

    .hi-tasks__board,
    .hi-tasks__loading {
      gap: 10px;
      padding: 12px 14px 14px;
    }

    .hi-tasks__total {
      display: none;
    }
  }
`;
