// purpose: 任务 — the task ledger as one board: todo / doing / serving / done / cancelled.
// The full canvas is organized by the one durable lifecycle. `serving` is its own column
// because a duty being kept up is not work in progress: it has no finish, so a Done button
// on it asks the wrong question, and its age means nothing while "last confirmed alive"
// means everything. Liveness detail is how a duty is checked; the status is what makes it
// one.
//
// The board is the whole page — five columns, each scrolling on its own, nothing
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
    servingN: (n) => `${n} serving`,
    totalN: (n) => `${n} ${n === 1 ? "task" : "tasks"}`,
    board: "Task board",
    category: {
      todo: "Todo",
      doing: "Doing",
      serving: "Serving",
      done: "Done",
      cancelled: "Cancelled",
    },
    empty: {
      todo: "Nothing queued.",
      doing: "Nothing in progress.",
      serving: "No standing duties.",
      done: "Nothing finished yet.",
      cancelled: "Nothing cancelled.",
    },
    attentionN: (n) => `${n} need attention`,
    created: (at) => `Created ${at}`,
    completed: (at) => `Completed ${at}`,
    cancelled: (at) => `Cancelled ${at}`,
    due: (at) => `Due ${at}`,
    overdue: (at) => `Overdue ${at}`,
    checked: (at) => `Alive ${at}`,
    neverChecked: "Never confirmed alive",
    start: "Start",
    serve: "Keep as a standing duty",
    moveTodo: "Move to todo",
    markDone: "Mark done",
    standDown: "Stand down",
    cancel: "Cancel",
    reopen: "Reopen",
    // Short forms for the card, where a column is a few hundred pixels wide. The
    // full wording above stays as the button's label for anyone not reading pixels.
    shortStart: "Start",
    shortServe: "Serve",
    shortTodo: "Todo",
    shortDone: "Done",
    shortStandDown: "Stand down",
    shortCancel: "Cancel",
    shortReopen: "Reopen",
    details: "Open details",
    close: "Close",
    malformed: "This task has invalid stored fields. Changing its status will rewrite the recognized fields.",
    malformedShort: "Invalid fields",
    noBody: "(no notes)",
    asked: "What was asked",
    assumed: "assumed, never confirmed",
    timeline: "What has happened",
    noTimeline: "Nothing recorded yet.",
    fullNotes: "Full notes",
    moment: {
      asked: "asked",
      landed: "landed",
      blocked: "blocked",
      checked: "checked",
      moved: "moved",
      note: "note",
    },
    monitoring: "Liveness",
    verify: "Check",
    restart: "If it stops",
    owner: "Owner",
    startKey: "Start key",
    onIt: "Who is on it",
    working: (session, ago) => `${session} — working ${ago}`,
    idling: (session, ago) => `${session} — idle ${ago}`,
    nobody: "Nobody on it",
    reopening: "The restart cut its worker off — it is being reopened",
    lost: "The restart took its worker and its session would not reopen",
    turnFailed: (why) => `last turn failed: ${why}`,
    turnStopped: "last turn was stopped",
    ago: (n, unit) => `${n}${unit} ago`,
    agoUnits: { m: "m", h: "h", d: "d" },
    justNow: "just now",
  },
  zh: {
    title: "任务",
    activeN: (n) => `${n} 件进行中`,
    servingN: (n) => `${n} 项值守`,
    totalN: (n) => `${n} 件任务`,
    board: "任务看板",
    category: {
      todo: "待办",
      doing: "进行中",
      serving: "值守",
      done: "已完成",
      cancelled: "已取消",
    },
    empty: {
      todo: "没有待办。",
      doing: "没有进行中的。",
      serving: "没有值守中的事。",
      done: "还没有完成的。",
      cancelled: "没有已取消的。",
    },
    attentionN: (n) => `${n} 件需要留意`,
    created: (at) => `创建于 ${at}`,
    completed: (at) => `完成于 ${at}`,
    cancelled: (at) => `取消于 ${at}`,
    due: (at) => `截止 ${at}`,
    overdue: (at) => `已逾期 ${at}`,
    checked: (at) => `${at} 确认在跑`,
    neverChecked: "从未确认在跑",
    start: "开始",
    serve: "转为长期值守",
    moveTodo: "移回待办",
    markDone: "完成",
    standDown: "撤下值守",
    cancel: "取消",
    reopen: "重新打开",
    shortStart: "开始",
    shortServe: "值守",
    shortTodo: "待办",
    shortDone: "完成",
    shortStandDown: "撤下",
    shortCancel: "取消",
    shortReopen: "重开",
    details: "查看详情",
    close: "关闭",
    malformed: "这条任务包含无效字段。修改状态时会重写可识别的字段。",
    malformedShort: "字段无效",
    noBody: "（没有备注）",
    asked: "要什么",
    assumed: "推断的，未经确认",
    timeline: "发生了什么",
    noTimeline: "还没有记录。",
    fullNotes: "完整记录",
    moment: {
      asked: "要求",
      landed: "交付",
      blocked: "受阻",
      checked: "核查",
      moved: "状态",
      note: "备注",
    },
    monitoring: "运行检查",
    verify: "检查方式",
    restart: "停止后",
    owner: "负责人",
    startKey: "启动标识",
    onIt: "谁在做",
    working: (session, ago) => `${session} — 工作中 ${ago}`,
    idling: (session, ago) => `${session} — 空闲 ${ago}`,
    nobody: "没有人在做",
    reopening: "重启中断了它的 worker，正在恢复",
    lost: "重启带走了它的 worker，会话无法恢复",
    turnFailed: (why) => `上一轮失败：${why}`,
    turnStopped: "上一轮被中止",
    ago: (n, unit) => `${n}${unit}前`,
    agoUnits: { m: "分钟", h: "小时", d: "天" },
    justNow: "刚刚",
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
  { id: "serving", label: L.category.serving, tone: "serving" },
  { id: "done", label: L.category.done, tone: "secondary" },
  { id: "cancelled", label: L.category.cancelled, tone: "danger" },
];

// A running-record line is read by its kind before its text, so the kind carries the
// colour: blocked is the one that should catch an eye crossing the panel, landed the one
// that says this went well, and `moved` — written by the store, not by a mind — is
// deliberately the quietest thing on the list.
const MOMENT_TONE = {
  asked: "var(--accent-2)",
  landed: "var(--accent-2)",
  blocked: "var(--danger)",
  checked: "var(--task-serving)",
  moved: "var(--fg-mute)",
  note: "var(--fg-mute)",
};

const STATUS_TONE = {
  todo: "var(--fg-dim)",
  doing: "var(--accent)",
  serving: "var(--task-serving)",
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
        <Header activeCount={0} servingCount={0} totalCount={0} />
        <div className="hi-tasks__loading" aria-label={L.title}>
          <span />
          <span />
          <span />
          <span />
          <span />
        </div>
      </div>
    );
  }

  // Counted apart, because they answer different questions. "3 active" that silently
  // included two permanent watches said there was more work on than there was, and the
  // number moved only when a duty was retired.
  const activeCount = tasks.filter((task) => task.status === "todo" || task.status === "doing").length;
  const servingCount = tasks.filter((task) => task.status === "serving").length;
  // A task whose status changed out from under the panel keeps the panel open on it —
  // it is still the task someone was reading. Only a task that left the ledger closes it.
  const open = openSubject ? tasks.find((task) => task.subject === openSubject) || null : null;

  return (
    <div className="hi-tasks">
      <style>{CSS}</style>
      <Header
        activeCount={activeCount}
        servingCount={servingCount}
        totalCount={tasks.length}
      />

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

function Header({ activeCount, servingCount, totalCount }) {
  return (
    <header className="hi-tasks__header">
      <div className="hi-tasks__heading">
        <h1>{L.title}</h1>
        <span>{L.activeN(activeCount)}</span>
        {servingCount > 0 && (
          <span className="hi-tasks__heading-serving">{L.servingN(servingCount)}</span>
        )}
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
  // The last thing that happened, clamped to one line. A board of titles says what each
  // task *is* and nothing about where any of it stands, which is the question somebody
  // scanning this actually has.
  const latest = latestMoment(task);
  // The other half of "does this need me": the running record says where the work got to,
  // this says whether anyone is still carrying it. A `doing` card with neither is the state
  // this line exists to make visible.
  const who = whoMeta(task);
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
        {latest && (
          <span
            className="hi-tasks__card-latest"
            style={{ "--moment": MOMENT_TONE[latest.kind] || "var(--fg-mute)" }}
          >
            <span className="hi-tasks__moment-kind">{L.moment[latest.kind] || latest.kind}</span>
            <span className="hi-tasks__card-latest-text">{latest.text}</span>
          </span>
        )}
        {who && (
          <span className="hi-tasks__card-who" data-warn={who.warn ? "true" : undefined}>
            {who.text}
          </span>
        )}
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
  // What they asked for is pinned rather than scrolled to: it is the first thing somebody
  // catching up on their own errand wants, and it does not move for the life of the task.
  // It is a **reading, not a gate** — nothing here waits on it and no task is held open
  // against it; showing it is what makes a wrong reading cheap to correct in one sentence.
  const asked = (task.timeline || []).find((moment) => moment.kind === "asked");
  // Newest first. The file appends, because that is what a writer with a shell can do
  // safely; a reader catching up wants the opposite order.
  const moments = [...(task.timeline || [])].reverse();
  // What was asked, then who is carrying it now, then what has happened — the three
  // questions a person catching up on their own errand asks, in that order.
  const who = whoMeta(task);
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

          {asked && (
            <div className="hi-tasks__asked">
              <div className="hi-tasks__asked-title">{L.asked}</div>
              <div className="hi-tasks__asked-text">{asked.text}</div>
            </div>
          )}

          {who && (
            <div className="hi-tasks__who" data-warn={who.warn ? "true" : undefined}>
              <div className="hi-tasks__who-title">{L.onIt}</div>
              <div className="hi-tasks__who-text">{who.text}</div>
              {who.detail && <div className="hi-tasks__who-detail">{who.detail}</div>}
            </div>
          )}

          <div className="hi-tasks__moments-title">{L.timeline}</div>
          {moments.length === 0 ? (
            <div className="hi-tasks__none">{L.noTimeline}</div>
          ) : (
            <ol className="hi-tasks__moments">
              {moments.map((moment, index) => (
                <li
                  key={`${moment.at || ""}-${index}`}
                  className="hi-tasks__moment"
                  style={{ "--moment": MOMENT_TONE[moment.kind] || "var(--fg-mute)" }}
                >
                  <span className="hi-tasks__moment-head">
                    <span className="hi-tasks__moment-kind">
                      {L.moment[moment.kind] || moment.kind}
                    </span>
                    {moment.at && (
                      <span className="hi-tasks__moment-at">{formatStamp(moment.at)}</span>
                    )}
                  </span>
                  <span className="hi-tasks__moment-text">
                    {inline(moment.text, `m${index}`)}
                  </span>
                </li>
              ))}
            </ol>
          )}

          {task.body ? (
            <details className="hi-tasks__notes">
              <summary>{L.fullNotes}</summary>
              <Prose text={task.body} />
            </details>
          ) : null}

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
//
// A card carries three buttons at most; the fourth transition on work — "keep this as a
// standing duty" — is a decision, not a flick, and it lives in the detail panel where
// there is room to name it. That panel is reachable by keyboard and by touch, so drag
// stays the shortcut it was always meant to be rather than the only way into `serving`.
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

  // A duty has no "done" to offer: it ends by being stood down, which is the same close
  // wearing the name of what actually happened.
  if (task.status === "serving") {
    return (
      <div className="hi-tasks__actions" draggable={false}>
        {button("ghost", "todo", L.moveTodo, L.shortTodo)}
        {button("danger", "cancelled", L.cancel, L.shortCancel)}
        {button("primary", "done", L.standDown, L.shortStandDown)}
      </div>
    );
  }

  return (
    <div className="hi-tasks__actions" draggable={false}>
      {task.status === "todo"
        ? button("ghost", "doing", L.start, L.shortStart)
        : button("ghost", "todo", L.moveTodo, L.shortTodo)}
      {!short && button("ghost", "serving", L.serve, L.shortServe)}
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

// The last line of the running record, whatever kind it is. `moved` counts: "this went
// to done an hour ago" is exactly as much news as anything a mind wrote.
function latestMoment(task) {
  const timeline = task.timeline || [];
  return timeline.length > 0 ? timeline[timeline.length - 1] : null;
}

function dueMeta(task) {
  if (!task.dueAt) return null;
  const time = new Date(task.dueAt).getTime();
  if (Number.isNaN(time)) return null;
  const overdue =
    (task.status === "todo" || task.status === "doing" || task.status === "serving") &&
    time <= Date.now();
  return {
    text: overdue ? L.overdue(formatStamp(task.dueAt)) : L.due(formatStamp(task.dueAt)),
    warn: overdue,
  };
}

// How long ago, in the spelling `tasks::ago` uses on the agent's side of the same fact.
// Relative rather than absolute on purpose: "idle 12m" is the question a reader of this line
// has, and an absolute stamp makes them do the subtraction.
function formatAgo(value) {
  const then = new Date(value).getTime();
  if (Number.isNaN(then)) return null;
  const mins = Math.max(0, Math.round((Date.now() - then) / 60000));
  if (mins < 1) return L.justNow;
  if (mins < 60) return L.ago(mins, L.agoUnits.m);
  if (mins < 60 * 24) return L.ago(Math.floor(mins / 60), L.agoUnits.h);
  return L.ago(Math.floor(mins / (60 * 24)), L.agoUnits.d);
}

// Who is on this task, or — where nobody is a problem — that nobody is.
//
// The switchboard join arrives on the task (`onIt`), computed by the same code that writes
// the agent's own window, so the board and the agent cannot disagree about it. The judgment
// left here is the one the server deliberately did not make: what an *absence* means.
//
// **"Nobody" is only said on `doing`.** A `todo` with no worker is what a `todo` is, and a
// duty spends most of its life with no live handler because one is spawned per burst and
// idles out. Printing it on those would put the phrase on most of the board and teach the eye
// to skip it — and then it would be skipped on the one card where it means something.
function whoMeta(task) {
  const on = task.onIt;
  if (!on) {
    return task.status === "doing" ? { text: L.nobody, warn: true } : null;
  }
  if (on.state === "reopening") {
    // No move is called for: the errand is coming back on its own, and an alarm here is what
    // made the seconds after every restart read as abandoned work.
    return { text: L.reopening, warn: false };
  }
  if (on.state === "lost") return { text: L.lost, warn: true };
  const since = on.since ? formatAgo(on.since) : "";
  const text = on.busy
    ? L.working(on.session, since)
    : L.idling(on.session, since);
  // `idle` after a turn that died says the wrong thing loudest — it is the same word a worker
  // waiting for its next instruction reports, so the detail is what separates the two.
  const detail = on.failed
    ? L.turnFailed(on.failed)
    : on.stopped
      ? L.turnStopped
      : on.doing || null;
  return { text, warn: Boolean(on.failed), detail };
}

// Every duty gets this line, including one with no `liveness` recorded — a duty nobody
// wrote a check for is the worse case, not an exempt one, and silence there reads as fine.
function healthMeta(task) {
  if (task.status !== "serving") return null;
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
  } else if (task.status === "serving") {
    // Never "Created Aug 3": a watch is supposed to be old, so its age is the one fact
    // here that means nothing. Whether it is still up is the only one that does.
    if (health && !notes.includes(health)) notes.push(health);
  } else if (task.createdAt) {
    notes.push({ text: L.created(formatStamp(task.createdAt)) });
  }
  return notes;
}

// What is left of a body once `split_timeline` has taken the dated lines out of it: the
// long prose, behind the fold. The timeline is the spine and this is everything the spine
// does not carry — and for now it is most of what there is, since nothing backfills, so a
// record written before the schema has its entire account down here.
//
// It was rendered into one flat block, which is survivable at a sentence or two and is not
// what these are: the live store's median body is 3.2 KB and its largest 48 KB. So it is
// read as the markdown the agent already writes.
//
// **A subset, deliberately.** Headings, bullets, bold, emphasis, inline code, and bare URLs
// — the whole vocabulary these bodies use. No images, tables or raw HTML: nothing here needs
// them, and each is a way for text a session wrote to reach further than text should.
// Output is React elements and never `dangerouslySetInnerHTML`, so an unclosed tag in
// someone's note stays a character on the screen instead of becoming markup.
//
// **URLs autolink; markdown links do not exist here.** These two are not the same
// concession. `[click here](somewhere-else)` lets text a session wrote name one destination
// and go to another, which is the reach the paragraph above refuses. An autolink cannot:
// the visible text *is* the href, so the worst it can do is take you where it plainly says.
// And a URL is exactly what a row blocked on the person has to carry — the panel has no
// address bar, so an inert one is a URL they must retype off their own screen.
//
// Underscores are *not* emphasis, and that is the one deliberate omission: these bodies are
// full of `status_since`, `checked_at`, `start_key`, and a renderer that reads those as
// italics mangles the field names the record is mostly made of.
//
// A marker must also be followed by a non-space to open, which is what stops "2 * 3 = 6"
// and a line carrying two loose asterisks from being read as one long emphasis.
const INLINE = /(\*\*[^*\s][^*]*?\*\*|`[^`]+`|\*[^*\s][^*\n]*\*)/g;
const AUTOLINK = /(https?:\/\/[^\s<>"'`]+)/g;

// A URL written mid-sentence collects the sentence's punctuation, and a URL written inside
// brackets collects the closing one. Neither belongs to the address: trailing `.,;:!?` always
// comes off, and a `)` comes off only when the run has no `(` to match it — which keeps the
// query strings and the wiki paths that genuinely carry balanced parens intact.
const count = (text, ch) => text.split(ch).length - 1;

function address(url) {
  let end = url.length;
  while (end > 0) {
    const last = url[end - 1];
    if (".,;:!?".includes(last)) {
      end -= 1;
      continue;
    }
    const body = url.slice(0, end);
    if ((last === ")" || last === "]") && count(body, last) > count(body, last === ")" ? "(" : "[")) {
      end -= 1;
      continue;
    }
    break;
  }
  return url.slice(0, end);
}

// Autolink only — never a label with a separate destination. The anchor's text is the href
// it goes to, so text the agent wrote cannot say one place and send the reader to another.
function linked(text, keyBase) {
  const out = [];
  let i = 0;
  for (const part of String(text).split(AUTOLINK)) {
    if (!part) continue;
    const key = `${keyBase}-u${i++}`;
    if (!/^https?:\/\//.test(part)) {
      out.push(part);
      continue;
    }
    const url = address(part);
    out.push(
      <a
        key={key}
        className="hi-tasks__link"
        href={url}
        target="_blank"
        rel="noreferrer noopener"
        onClick={(event) => event.stopPropagation()}
      >
        {url}
      </a>,
    );
    if (url.length < part.length) out.push(part.slice(url.length));
  }
  return out;
}

// Code spans are the one run left alone: a URL somebody deliberately fenced is being shown
// as a literal, and the panel is full of `start_key`-shaped strings that are not addresses.
function inline(text, keyBase) {
  const out = [];
  let i = 0;
  for (const part of String(text).split(INLINE)) {
    if (!part) continue;
    const key = `${keyBase}-${i++}`;
    if (part.length > 4 && part.startsWith("**") && part.endsWith("**")) {
      out.push(<strong key={key}>{linked(part.slice(2, -2), key)}</strong>);
    } else if (part.length > 2 && part.startsWith("`") && part.endsWith("`")) {
      out.push(<code key={key}>{part.slice(1, -1)}</code>);
    } else if (part.length > 2 && part.startsWith("*") && part.endsWith("*")) {
      out.push(<em key={key}>{linked(part.slice(1, -1), key)}</em>);
    } else {
      out.push(...linked(part, key));
    }
  }
  return out;
}

// Paragraph lines are joined with a space rather than kept as typed. Prose written at a
// terminal is hard-wrapped around column 90, and preserving those breaks at panel width
// gives every paragraph a ragged right edge that reads as damage. A blank line still
// separates paragraphs, which is the break that carries meaning.
function blocks(text) {
  const out = [];
  let para = [];
  let list = null;
  const flushPara = () => {
    if (para.length) out.push({ kind: "p", text: para.join(" ") });
    para = [];
  };
  const flushList = () => {
    if (list) out.push({ kind: "ul", items: list });
    list = null;
  };
  for (const raw of String(text).replace(/\r\n?/g, "\n").split("\n")) {
    const line = raw.replace(/\s+$/, "");
    if (!line.trim()) {
      flushPara();
      flushList();
      continue;
    }
    const heading = /^(#{1,6})\s+(.*)$/.exec(line);
    if (heading) {
      flushPara();
      flushList();
      out.push({ kind: "h", depth: heading[1].length, text: heading[2] });
      continue;
    }
    const bullet = /^\s*[-*+]\s+(.*)$/.exec(line);
    if (bullet) {
      flushPara();
      if (!list) list = [];
      list.push(bullet[1]);
      continue;
    }
    // An indented line under a bullet continues it; anything else is prose.
    if (list && /^\s{2,}\S/.test(line)) {
      list[list.length - 1] += ` ${line.trim()}`;
      continue;
    }
    flushList();
    para.push(line);
  }
  flushPara();
  flushList();
  return out;
}

function Prose({ text }) {
  return (
    <div className="hi-tasks__prose">
      {blocks(text).map((block, i) => {
        if (block.kind === "h") {
          return (
            <div key={i} className="hi-tasks__prose-h" data-depth={Math.min(block.depth, 3)}>
              {inline(block.text, i)}
            </div>
          );
        }
        if (block.kind === "ul") {
          return (
            <ul key={i} className="hi-tasks__prose-ul">
              {block.items.map((item, j) => (
                <li key={j}>{inline(item, `${i}-${j}`)}</li>
              ))}
            </ul>
          );
        }
        return (
          <p key={i} className="hi-tasks__prose-p">
            {inline(block.text, i)}
          </p>
        );
      })}
    </div>
  );
}

const CSS = `
  .hi-tasks,
  .hi-tasks * {
    box-sizing: border-box;
  }

  /* Serving needs a hue of its own, and the palette's four are spoken for: terracotta is
     doing, sage is done, red is cancelled, ink-dim is todo. A cool slate is the one thing
     left that cannot be mistaken for any of them in either mode — and it reads steady
     rather than urgent, which is what a duty that is up should read as. Scoped here
     because it means something only on this board. */
  .hi-tasks {
    --task-serving: #5a7a99;
  }

  @media (prefers-color-scheme: dark) {
    :root:not([data-theme="light"]) .hi-tasks {
      --task-serving: #8fb0cc;
    }
  }

  :root[data-theme="dark"] .hi-tasks {
    --task-serving: #8fb0cc;
  }

  /* No ground of its own: the board stands on the layer's paper, which runs under
     the safe padding to the window edge. A flat colour here would fill only the
     padded content box and frame the board in the paper it doesn't match. */
  .hi-tasks {
    position: relative;
    width: 100%;
    height: 100%;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
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
    /* Nothing is reserved for the window's titlebar any more, so the heading clears
       it here: 0 in a browser tab, the strip in the desktop window, the notch on a
       phone. */
    padding: max(14px, var(--hi-safe-top)) clamp(16px, 2.4vw, 30px) 14px;
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

  .hi-tasks__heading .hi-tasks__heading-serving {
    color: var(--task-serving);
  }

  /* The board is the page: five columns that fill the height and never scroll it.
     Below ~1300px the columns keep a readable floor and the board scrolls sideways
     instead of squeezing every card into a ribbon. */
  .hi-tasks__board {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: repeat(5, minmax(236px, 1fr));
    gap: 12px;
    /* A board is the case the bottom token exists for: a card half under a control
       disc is a lost row, not texture passing behind glass. */
    padding: 14px clamp(16px, 2.4vw, 30px) calc(16px + var(--hi-chrome-bottom));
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
  .hi-tasks__column[data-tone="serving"] { --column-tone: var(--task-serving); }
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

  /* One line, clamped: a card is a glance, and a second line of record would cost the
     column a card. The panel is where the rest of it is. */
  .hi-tasks__card-latest {
    display: flex;
    align-items: baseline;
    gap: 6px;
    margin-top: 6px;
    min-width: 0;
  }

  .hi-tasks__card-latest-text {
    flex: 1;
    min-width: 0;
    color: var(--fg-dim);
    font-size: 11.5px;
    line-height: 1.4;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* One clipped line, under the record and above the dates: it is a fact about right now,
     and the two things around it are about the past. */
  .hi-tasks__card-who {
    min-width: 0;
    color: var(--fg-dim);
    font-size: 11.5px;
    line-height: 1.4;
    font-weight: 620;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .hi-tasks__card-who[data-warn="true"] {
    color: var(--danger);
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

  /* Pinned above the record and never scrolled past: one or three lines in their own
     words, so it can be read as prose rather than parsed as a field. */
  .hi-tasks__asked {
    margin-bottom: 16px;
    padding: 11px 13px;
    border-left: 3px solid var(--accent-2);
    background: color-mix(in srgb, var(--accent-2) 8%, transparent);
  }

  .hi-tasks__asked-title {
    margin-bottom: 4px;
    color: var(--fg-dim);
    font-size: 11.5px;
    font-weight: 750;
  }

  .hi-tasks__asked-text {
    color: var(--fg);
    font-size: 13.5px;
    line-height: 1.6;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  /* Between what was asked and what has happened, because that is where it belongs in
     time: the only line on this panel about the present. */
  .hi-tasks__who {
    margin-bottom: 16px;
    padding: 9px 12px;
    border-left: 3px solid var(--fg-mute);
    background: color-mix(in srgb, var(--fg-mute) 7%, transparent);
  }

  .hi-tasks__who[data-warn="true"] {
    border-left-color: var(--danger);
    background: color-mix(in srgb, var(--danger) 7%, transparent);
  }

  .hi-tasks__who-title {
    margin-bottom: 4px;
    color: var(--fg-dim);
    font-size: 11.5px;
    font-weight: 750;
  }

  .hi-tasks__who-text {
    color: var(--fg);
    font-size: 13px;
    line-height: 1.5;
    overflow-wrap: anywhere;
  }

  .hi-tasks__who[data-warn="true"] .hi-tasks__who-text {
    color: var(--danger);
  }

  .hi-tasks__who-detail {
    margin-top: 3px;
    color: var(--fg-dim);
    font-size: 12px;
    line-height: 1.5;
    overflow-wrap: anywhere;
  }

  .hi-tasks__moments-title {
    margin-bottom: 8px;
    color: var(--fg-dim);
    font-size: 11.5px;
    font-weight: 750;
  }

  .hi-tasks__moments {
    margin: 0 0 4px;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 9px;
  }

  .hi-tasks__moment {
    display: grid;
    grid-template-columns: auto 1fr;
    align-items: baseline;
    gap: 4px 10px;
    padding-left: 11px;
    border-left: 2px solid color-mix(in srgb, var(--moment) 55%, transparent);
  }

  .hi-tasks__moment-head {
    display: flex;
    align-items: baseline;
    gap: 7px;
    white-space: nowrap;
  }

  .hi-tasks__moment-kind {
    color: var(--moment);
    font-size: 11px;
    font-weight: 780;
    letter-spacing: 0.02em;
  }

  .hi-tasks__moment-at {
    color: var(--fg-mute);
    font-size: 11px;
    font-weight: 620;
  }

  .hi-tasks__moment-text {
    color: var(--fg-dim);
    font-size: 13px;
    line-height: 1.6;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  /* The long account is still here and still whole — just not the first thing between a
     person and what happened. Live bodies run to tens of kilobytes of working notes. */
  .hi-tasks__notes {
    margin-top: 14px;
    border-top: 1px solid var(--line);
    padding-top: 10px;
  }

  .hi-tasks__notes > summary {
    cursor: pointer;
    color: var(--fg-dim);
    font-size: 11.5px;
    font-weight: 750;
  }

  .hi-tasks__notes > summary:focus-visible {
    outline: 3px solid var(--accent-soft);
    outline-offset: 2px;
  }

  .hi-tasks__notes .hi-tasks__prose {
    margin-top: 9px;
  }

  .hi-tasks__prose,
  .hi-tasks__none {
    color: var(--fg-dim);
    font-size: 13.5px;
    line-height: 1.65;
    overflow-wrap: anywhere;
  }

  /* No white-space: pre-wrap on the prose any more. blocks() decides where the breaks
     are, and leaving it on would keep the source's hard wraps as well and double them. */
  .hi-tasks__prose-p {
    margin: 0 0 10px;
  }

  .hi-tasks__prose > :last-child {
    margin-bottom: 0;
  }

  /* One weight of heading, three sizes. A record's sections are shallow — the depth is
     never the point, only that a section started — so this reads the marker without
     pretending a level-3 heading means something a level-6 does not. */
  .hi-tasks__prose-h {
    margin: 14px 0 6px;
    color: var(--fg);
    font-weight: 700;
    line-height: 1.4;
  }

  .hi-tasks__prose-h[data-depth="1"] { font-size: 14.5px; }
  .hi-tasks__prose-h[data-depth="2"] { font-size: 13.5px; }
  .hi-tasks__prose-h[data-depth="3"] { font-size: 13px; }

  .hi-tasks__prose > .hi-tasks__prose-h:first-child {
    margin-top: 0;
  }

  .hi-tasks__prose-ul {
    margin: 0 0 10px;
    padding-left: 18px;
    list-style: disc;
  }

  .hi-tasks__prose-ul li {
    margin-bottom: 3px;
  }

  .hi-tasks__prose strong {
    color: var(--fg);
    font-weight: 700;
  }

  .hi-tasks__link {
    color: var(--accent);
    text-decoration: underline;
    text-decoration-color: var(--accent-line);
    text-underline-offset: 2px;
    overflow-wrap: anywhere;
  }

  .hi-tasks__link:hover {
    text-decoration-color: var(--accent);
  }

  .hi-tasks__prose code {
    padding: 1px 4px;
    border-radius: 4px;
    background: color-mix(in srgb, var(--fg) 8%, transparent);
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 0.92em;
  }

  .hi-tasks__liveness {
    margin-top: 14px;
    padding: 12px 14px;
    border-left: 3px solid var(--task-serving);
    background: color-mix(in srgb, var(--task-serving) 7%, transparent);
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
    grid-template-columns: repeat(5, minmax(236px, 1fr));
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
