// purpose: 任务 — the task ledger, weighted by what is still owed. Three columns of live
// work — todo / doing / serving — and one ledger rail for everything that has closed.
// `serving` is its own column because a duty being kept up is not work in progress: it has
// no finish, so a Done button on it asks the wrong question, and its age means nothing
// while "last confirmed alive" means everything.
//
// **Five equal columns were the defect this replaced, and it was a defect of proportion.**
// On the live store that exposed it, 130 of 140 rows were closed: done and cancelled held
// 93% of the ledger, 40% of the width, and — because every closed card printed the word its
// own column already said, a stamp, and a Reopen button nobody presses — they were also the
// busiest thing on the screen. The nine rows that were actually owed sat in three columns
// that were mostly air. Reading it meant searching the quiet half for the point. Closed work
// is not hidden now, it is *thin*: one line each in a rail, newest first, no verb of its own.
//
// So the canvas is spent on what is unfinished, and the rail carries what is done in the
// space one line deserves. Nothing is behind a filter; the archive is simply not shouting.
//
// A card is a *glance*: a clamped title, the **one** fact that decides whether it needs you,
// and at most one verb. Everything long — the untruncated title, the prose, the liveness
// contract, every other transition — lives in the detail panel a card opens. A clipped
// sentence is not one of the facts: "进展 The current frontier carries no n…" costs a line
// and cannot be read, so prose reaches the card only when it is a *wait*, which is the one
// case where the fragment is what tells you to act.
//
// A status change is a card moved between columns, and dragging one there is the gesture
// the layout already promises — the rail takes a drop too, which is how you finish something
// without opening it. It is never the *only* way: HTML5 drag does not exist on touch and
// cannot be driven from a keyboard, so the primary verb stays on the card and every
// remaining transition is in the panel, which opens by click, tap and Enter alike.
import { useState, useEffect, useLayoutEffect, useCallback, useRef } from "react";
import { useLive, TEMPO } from "@hi/core";

const J = { "Content-Type": "application/json" };
const api = {
  list: () => fetch("/api/tasks").then((response) => response.json()),
  // The switchboard, joined to the ledger by subject in this view rather than served
  // alongside it. `GET /api/workers` already carries `subject` — the workers page reads it
  // the same way — so the alternative was a field on every row of a polled list, its own
  // staleness rules, and tests keeping both derivations agreeing forever, for a fact one
  // lookup answers. What that costs is precision the person's board does not spend: a
  // restart's casualties are not in the roster at all, so cut-off work reads "nobody on it"
  // rather than naming the restart. Still true, still the same alarm; resume-or-write-off is
  // the agent's decision, made against `tasks::worker_note`, not this one.
  workers: () => fetch("/api/workers").then((response) => response.json()),
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
    board: "Task board",
    closed: "Closed",
    ledger: "Closed work",
    dropDone: "Drop to finish",
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
      closed: "Nothing has closed yet.",
    },
    attentionN: (n) => `${n} need attention`,
    needsYouN: (n) => `${n} waiting on you`,
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
    // The one label a card is too narrow for in full. Every other verb the card shows is
    // already one word; the panel has room and says what the click means.
    shortDone: "Done",
    details: "Open details",
    close: "Close",
    malformed: "This task has invalid stored fields. Changing its status will rewrite the recognized fields.",
    malformedShort: "Invalid fields",
    noBody: "(no notes)",
    wanted: "What you asked for",
    needsYou: "Needs you",
    assumed: "assumed, never confirmed",
    timeline: "What has happened",
    noTimeline: "Nothing recorded yet.",
    // One field, two questions, because the reader arrives with a different one
    // depending on whether the row is still owed.
    accountOpen: "Where it stands",
    accountClosed: "What came of it",
    showAll: "Show everything",
    showLess: "Show less",
    moment: {
      created: "created",
      update: "update",
      delivered: "delivered",
      waiting: "waiting",
      moved: "moved",
      note: "update",
      // Not kinds the store writes — the switchboard's line, built by `liveMoment`. The
      // word is deliberately not a past-tense one: everything else on this list happened,
      // and this is happening.
      live: "now",
      failed: "last turn failed",
    },
    // A status change reads as the verb for it. The stored text is the pair the store
    // wrote — `todo → doing` — which is the transition spelled out for a machine, and
    // the person reading it wants the word.
    life: {
      started: "started",
      putBack: "put back",
      reopened: "reopened",
      serving: "standing duty",
      done: "done",
      cancelled: "cancelled",
    },
    byHand: "by you",
    monitoring: "Liveness",
    verify: "Check",
    restart: "If it stops",
    owner: "Owner",
    startKey: "Start key",
    fields: "Other fields",
    moreFields: (n) => `+${n} more, in the record`,
    standing: (label, span) => `${label} for ${span}`,
    agoUnits: { m: "m", h: "h", d: "d" },
    ago: (span) => `${span} ago`,
    nobodyOnIt: "Nobody on it",
    onItRunning: (span) => `Running for ${span}`,
    onItIdle: (span) => `Worker idle ${span}`,
  },
  zh: {
    title: "任务",
    activeN: (n) => `${n} 件进行中`,
    servingN: (n) => `${n} 项值守`,
    board: "任务看板",
    closed: "已结束",
    ledger: "已结束的任务",
    dropDone: "拖到这里＝完成",
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
      closed: "还没有结束的任务。",
    },
    attentionN: (n) => `${n} 件需要留意`,
    needsYouN: (n) => `${n} 件等你`,
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
    shortDone: "完成",
    details: "查看详情",
    close: "关闭",
    malformed: "这条任务包含无效字段。修改状态时会重写可识别的字段。",
    malformedShort: "字段无效",
    noBody: "（没有备注）",
    wanted: "你要什么",
    needsYou: "等你处理",
    assumed: "推断的，未经确认",
    timeline: "发生了什么",
    noTimeline: "还没有记录。",
    accountOpen: "进展如何",
    accountClosed: "结果如何",
    showAll: "展开全部",
    showLess: "收起",
    moment: {
      created: "创建",
      update: "进展",
      delivered: "交付",
      waiting: "等人",
      moved: "状态",
      note: "进展",
      live: "此刻",
      failed: "上一轮失败",
    },
    life: {
      started: "开始",
      putBack: "退回待办",
      reopened: "重开",
      serving: "转为值守",
      done: "完成",
      cancelled: "取消",
    },
    byHand: "你改的",
    monitoring: "运行检查",
    verify: "检查方式",
    restart: "停止后",
    owner: "负责人",
    startKey: "启动标识",
    fields: "其他字段",
    moreFields: (n) => `还有 ${n} 条，在记录里`,
    standing: (label, span) => `${label} ${span}`,
    agoUnits: { m: "分钟", h: "小时", d: "天" },
    ago: (span) => `${span}前`,
    nobodyOnIt: "无人在做",
    onItRunning: (span) => `已跑 ${span}`,
    onItIdle: (span) => `执行者停了 ${span}`,
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

// The columns are the *unfinished* lifecycle. Done and cancelled are still on this page —
// in the rail, by time rather than by which of the two closes they were — because "what
// happened recently" is one question and splitting it across two columns answered it twice
// at forty percent of the width. Which close it was is a mark on the row and a word in the
// panel; it is not worth a column.
const LIVE_COLUMNS = [
  { id: "todo", label: L.category.todo, tone: "mute" },
  { id: "doing", label: L.category.doing, tone: "accent" },
  { id: "serving", label: L.category.serving, tone: "serving" },
];

const CLOSED = ["done", "cancelled"];

// A running-record line is read by its kind before its text, so the kind carries the
// colour: `waiting` is the one that should catch an eye crossing the panel, `delivered`
// the one that says this went well, and `moved` — written by the store, not by a mind —
// is deliberately the quietest thing on the list.
//
// **`update` is dim on purpose.** It is the default kind and it holds most of the record;
// a default that draws the eye is a record with no shape, and the reader is scanning for
// the two lines that mean something to them.
const MOMENT_TONE = {
  created: "var(--accent-2)",
  delivered: "var(--accent-2)",
  waiting: "var(--danger)",
  update: "var(--fg-dim)",
  moved: "var(--fg-mute)",
  note: "var(--fg-mute)",
  // The switchboard's two, which are the status colours and not record colours: `live` is
  // the same `--accent` the word `Doing` is drawn in, because that is what it qualifies.
  live: "var(--accent)",
  failed: "var(--danger)",
};

// A `waiting` line is a dated sentence in an append-only record, and nothing ever clears
// one. So the alarm colour is spent only where the wait is still true — newest — and a
// superseded one reads as the history it is. This is the confusion the rename was for: a
// red stripe on a row that had moved on three times since.
function momentTone(moment, live) {
  if (moment.kind === "waiting" && !live) return "var(--fg-mute)";
  return MOMENT_TONE[moment.kind] || "var(--fg-mute)";
}

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
  // Subject -> the session on it, from the same poll. `null` until the first roster lands, so
  // the board can tell "nobody is on this" from "we have not asked yet" and say neither early.
  const [onIt, setOnIt] = useState(null);
  // The card in hand: `{ subject, status }`, so a column can tell whether a drop over it
  // would change anything before it offers to accept one.
  const [drag, setDrag] = useState(null);
  // Refs, not the state: the poll interval is created once and would otherwise close
  // over the `busy` and `drag` of its first render.
  const busyRef = useRef(false);
  const dragRef = useRef(false);

  const reload = useCallback(async () => {
    const [data, roster] = await Promise.all([
      api.list().catch(() => ({ tasks: [] })),
      // A failed roster is *not* an empty one. Empty says nobody is on anything, which on
      // this board is the alarm on every `doing` row at once — so a fetch that did not come
      // back leaves the last reading standing rather than raising eleven false alarms.
      api.workers().catch(() => null),
    ]);
    setTasks(data.tasks || []);
    if (roster) setOnIt(bySubject(roster.workers || []));
  }, []);

  // Re-read, for the reason the workers roster does: the agent opens and closes tasks while
  // this is on screen, and a ledger that is quietly stale still reads as authoritative —
  // it is the surface someone checks *before* asking "did you drop that?". This is a ledger
  // rather than something you watch happen, so it runs on the slower tempo.
  //
  // The hold reads the refs and not the state: `busyRef` is set imperatively either side of
  // an `await`, and at that moment the matching `setBusy` has not committed, so a tick in
  // the gap would see the old value and flip a card back under the click that changed it.
  // Mid-drag it is the same shape — re-rendering the board out from under a held card
  // cancels the drag.
  useLive(reload, {
    period: TEMPO.ledger,
    hold: () => busyRef.current || dragRef.current,
  });

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
        <Header activeCount={0} servingCount={0} needsYou={0} />
        <div className="hi-tasks__loading" aria-label={L.title}>
          <span />
          <span />
          <span />
          <span data-rail="true" />
        </div>
      </div>
    );
  }

  // Counted apart, because they answer different questions. "3 active" that silently
  // included two permanent watches said there was more work on than there was, and the
  // number moved only when a duty was retired.
  const activeCount = tasks.filter((task) => task.status === "todo" || task.status === "doing").length;
  const servingCount = tasks.filter((task) => task.status === "serving").length;
  // The one count that is somebody else's move, so it is the one the heading leads with when
  // it is not zero. Every other number here says how much there is; this one says whether the
  // person reading has to do anything before any of it moves.
  const needsYou = tasks.filter(waitsOnPerson).length;
  // The roster read, carried on the row rather than threaded through every component that
  // draws one. `undefined` while no roster has landed, `null` once one has and nobody is on
  // this — the two say different things and `onItMeta` answers only the second.
  const rows = onIt
    ? tasks.map((task) => ({ ...task, onIt: onIt.get(task.subject) || null }))
    : tasks;
  // A task whose status changed out from under the panel keeps the panel open on it —
  // it is still the task someone was reading. Only a task that left the ledger closes it.
  const open = openSubject ? rows.find((task) => task.subject === openSubject) || null : null;

  return (
    <div className="hi-tasks">
      <style>{CSS}</style>
      <Header activeCount={activeCount} servingCount={servingCount} needsYou={needsYou} />

      <div className="hi-tasks__board" aria-label={L.board}>
        {LIVE_COLUMNS.map((column) => (
          <Column
            key={column.id}
            column={column}
            tasks={rows.filter((task) => task.status === column.id)}
            busy={busy}
            drag={drag}
            onStatus={setTaskStatus}
            onOpen={setOpenSubject}
            onDragStart={startDrag}
            onDragEnd={endDrag}
            onDrop={dropOn}
          />
        ))}
        <Ledger
          tasks={rows.filter((task) => CLOSED.includes(task.status))}
          busy={busy}
          drag={drag}
          onOpen={setOpenSubject}
          onDrop={dropOn}
        />
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

// The count of everything ever filed used to sit here, alone on the right, in the one
// position on the page the eye lands on first — and "140 tasks" is the least actionable
// number this surface knows. What is owed leads instead, and the size of the archive is a
// number on the archive.
function Header({ activeCount, servingCount, needsYou }) {
  return (
    <header className="hi-tasks__header">
      <div className="hi-tasks__heading">
        <h1>{L.title}</h1>
        {needsYou > 0 && (
          <span className="hi-tasks__heading-needs">{L.needsYouN(needsYou)}</span>
        )}
        <span>{L.activeN(activeCount)}</span>
        {servingCount > 0 && (
          <span className="hi-tasks__heading-serving">{L.servingN(servingCount)}</span>
        )}
      </div>
    </header>
  );
}

function Column({ column, tasks, busy, drag, onStatus, onOpen, onDragStart, onDragEnd, onDrop }) {
  const attention = tasks.filter(taskNeedsAttention).length;
  // A row that will not move until the person does goes to the top of its column. It was
  // drawn at the weight of everything else and in whatever order the ledger happened to
  // hold, which on the store that exposed this put the single task waiting on a human
  // fourth in the middle column. Sort is stable, so within each half the ledger's own
  // order survives — this only lifts, it does not reshuffle.
  const ordered = [...tasks].sort(
    (a, b) => Number(taskNeedsAttention(b)) - Number(taskNeedsAttention(a)),
  );
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
          {ordered.map((task) => (
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

// The archive, at the width one line is worth. Done and cancelled share one list, newest
// close first, because the question this half answers is *what has been happening* and the
// answer is chronological. Which of the two closes it was is the mark on the left and a word
// in the panel — it was never worth a column of its own, and as a column it doubled the cost
// of the answer.
//
// **No verb on a row.** Reopen was a hundred and thirty buttons for a transition taken
// perhaps twice a year, and it was the heaviest thing on every closed card. It lives in the
// panel a row opens, next to the record that makes reopening a decision rather than a
// mis-click.
//
// It still takes a drop, and only into `done`: finishing a card by throwing it at the
// archive is the gesture the five columns used to carry, and it is the one worth keeping.
// Cancelling is a judgment about work that did not happen, which is not a flick.
function Ledger({ tasks, busy, drag, onOpen, onDrop }) {
  const [over, setOver] = useState(false);
  const takes = Boolean(drag) && !CLOSED.includes(drag.status);
  const rows = [...tasks].sort((a, b) => closedStamp(b) - closedStamp(a));

  return (
    <section
      className="hi-tasks__ledger"
      data-drop={takes && over ? "true" : undefined}
      aria-label={`${L.ledger} (${tasks.length})`}
      onDragOver={(event) => {
        if (!takes) return;
        event.preventDefault();
        event.dataTransfer.dropEffect = "move";
        setOver(true);
      }}
      onDragLeave={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) setOver(false);
      }}
      onDrop={(event) => {
        if (!takes) return;
        event.preventDefault();
        setOver(false);
        onDrop(event.dataTransfer.getData("text/plain") || drag.subject, "done");
      }}
    >
      <div className="hi-tasks__column-head">
        <span className="hi-tasks__column-dot" aria-hidden="true" />
        <h2 className="hi-tasks__column-label">{L.closed}</h2>
        {takes && <span className="hi-tasks__ledger-hint">{L.dropDone}</span>}
        <span className="hi-tasks__column-count">{tasks.length}</span>
      </div>

      {rows.length === 0 ? (
        <div className="hi-tasks__empty">{L.empty.closed}</div>
      ) : (
        <div className="hi-tasks__ledger-rows">
          {rows.map((task) => (
            <button
              key={task.subject}
              type="button"
              className="hi-tasks__ledger-row"
              data-status={task.status}
              aria-busy={busy === task.subject}
              // One line clips hard, and these titles run to paragraphs — so the whole one
              // is on the hover, exactly as it is on a card.
              title={`${task.title || task.subject}\n\n${L.details}`}
              onClick={() => onOpen(task.subject)}
            >
              <span className="hi-tasks__ledger-mark" aria-hidden="true" />
              <span className="hi-tasks__ledger-title">{task.title || task.subject}</span>
              <span className="hi-tasks__ledger-when">{formatDay(closedStamp(task))}</span>
            </button>
          ))}
        </div>
      )}
    </section>
  );
}

function Card({ task, busy, dragging, onStatus, onOpen, onDragStart, onDragEnd }) {
  // **Exactly one line under the title**, and which line it is decides the card. A wait is
  // the only case where prose reaches the board: "this will not move until you do" is not
  // actionable without saying what is wanted, and the sentence is the cheapest true answer.
  // Everything else is a *note* — short, whole, and never a clipped sentence. The board used
  // to print the last record line whatever it was, which produced a column of fragments
  // ("进展 The current frontier carries no n…") that cost a line each and could not be read.
  const waiting = waitsOnPerson(task) ? latestSpoken(task) : null;
  const note = waiting ? null : cardNote(task);
  // A drag that ends on the card it started from still delivers a `click`, which would
  // open the panel on a gesture the person meant as "put it back".
  const dragged = useRef(false);

  return (
    <article
      className="hi-tasks__card"
      draggable
      data-malformed={task.malformed ? "true" : undefined}
      data-needs={waiting ? "true" : undefined}
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
        {waiting && (
          <span className="hi-tasks__card-wait">
            <span className="hi-tasks__card-wait-flag">{L.needsYou}</span>
            <span className="hi-tasks__card-wait-text">{waiting.text}</span>
          </span>
        )}
        {note && (
          <span className="hi-tasks__card-note" data-warn={note.warn ? "true" : undefined}>
            {note.text}
          </span>
        )}
      </button>

      <Actions task={task} busy={busy} onStatus={onStatus} card />
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
  const age = ageMeta(task);
  // Whether anybody is on this, beside the status: "somebody is on this" is a qualifier on the
  // word `Doing`. Only the fact and its clock live here — *what* the worker is doing is the
  // newest line of the record below, where it fits whole (see `liveMoment`).
  const on = onItMeta(task);
  const live = liveMoment(task);
  // What they asked for is pinned rather than scrolled to: it is the first thing somebody
  // catching up on their own errand wants, and it does not move for the life of the task.
  // It is a **reading, not a gate** — nothing here waits on it and no task is held open
  // against it; showing it is what makes a wrong reading cheap to correct in one sentence.
  const wanted = (task.timeline || []).find((moment) => moment.kind === "created");
  // The live wait, if there is one, so the reader learns whether to act before reading a
  // record to work it out. It is the same object as the line below in the list, which is
  // what lets that line keep the alarm colour while every older wait goes quiet.
  const waiting = waitsOnPerson(task) ? latestSpoken(task) : null;
  // A record spells its artifacts in inline code — *"the completed report is
  // `inspection-report.md` in this task directory"* — and that was a pointer the one
  // person reading it could not follow. The server says which of those names are really
  // on disk; this turns each one into the file itself, wherever the record says it.
  const linkFile = useCallback(
    (token) =>
      (task.files || []).some((file) => file.path === token)
        ? `/api/tasks/${encodeURIComponent(task.subject)}/files/${token
            .split("/")
            .map(encodeURIComponent)
            .join("/")}`
        : null,
    [task.subject, task.files],
  );
  // Newest first. The file appends, because that is what a writer with a shell can do
  // safely; a reader catching up wants the opposite order.
  const moments = [...(task.timeline || [])].reverse();
  // Frontmatter this schema does not know. The store keeps it because a writer that does not
  // understand a line is not entitled to drop it; the panel shows it for the same reason —
  // most of what a real record says about itself is down here, not in the twelve parsed keys.
  //
  // `systems` is promoted because it is the one that answers *what is this about*: it is on
  // 78 of one live store's 108 records and it reads as a tag, not as a field. That is a
  // presentation guess about one common spelling, deliberately not a schema — every other key
  // is listed exactly as written, in the file's order.
  const fields = task.extra || [];
  const systems = fields.find((field) => field.key === "systems" && !field.clipped);
  const rest = fields.filter((field) => field !== systems);
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
            {on && <span data-warn={on.warn ? "true" : undefined}>{on.text}</span>}
            {task.createdAt && <span>{L.created(formatStamp(task.createdAt))}</span>}
            {closedAt && <span>{closedAt}</span>}
            {age && <span data-warn={age.warn ? "true" : undefined}>{age.text}</span>}
            {due && <span data-warn={due.warn ? "true" : undefined}>{due.text}</span>}
            {health && <span data-warn={health.warn ? "true" : undefined}>{health.text}</span>}
          </div>

          {systems && (
            <div className="hi-tasks__systems">
              {systems.value
                .split(",")
                .map((name) => name.trim())
                .filter(Boolean)
                .map((name) => (
                  <span key={name} className="hi-tasks__system">
                    {name}
                  </span>
                ))}
            </div>
          )}

          {task.malformed && <div className="hi-tasks__bad">{L.malformed}</div>}

          {waiting && (
            <div className="hi-tasks__waiting">
              <div className="hi-tasks__waiting-title">{L.needsYou}</div>
              <div className="hi-tasks__waiting-text">
                {inline(waiting.text, "waiting", linkFile)}
              </div>
            </div>
          )}

          {wanted && (
            <div className="hi-tasks__asked">
              <div className="hi-tasks__asked-title">{L.wanted}</div>
              <div className="hi-tasks__asked-text">{inline(wanted.text, "wanted", linkFile)}</div>
            </div>
          )}

          {task.body && task.body.trim() ? (
            <Account
              text={task.body}
              link={linkFile}
              label={
                task.status === "done" || task.status === "cancelled"
                  ? L.accountClosed
                  : L.accountOpen
              }
            />
          ) : null}

          <div className="hi-tasks__moments-title">{L.timeline}</div>
          {moments.length === 0 && !live ? (
            <div className="hi-tasks__none">{L.noTimeline}</div>
          ) : (
            <ol className="hi-tasks__moments">
              {live && (
                <li
                  className="hi-tasks__moment"
                  data-live={live.kind}
                  style={{ "--moment": momentTone(live) }}
                >
                  <span className="hi-tasks__moment-head">
                    <span className="hi-tasks__moment-kind">{L.moment[live.kind]}</span>
                    {live.ago && <span className="hi-tasks__moment-at">{live.ago}</span>}
                  </span>
                  {/* Through `linked` and not `inline`, for the reason a field value is: this
                      is a command line the worker is running, so its backticks and asterisks
                      are its own characters and not markup. A URL in it is still worth a
                      click — the panel has no address bar. */}
                  <span className="hi-tasks__moment-text">{linked(live.text, "live")}</span>
                </li>
              )}
              {moments.map((moment, index) => {
                // A status change is shown as the word for it and nothing else: the stored
                // `todo → doing` is the transition spelled for a machine, and repeating
                // it beside the word would be the same fact twice.
                const life = moment.kind === "moved" ? lifecycleWord(moment.text) : null;
                return (
                  <li
                    key={`${moment.at || ""}-${index}`}
                    className="hi-tasks__moment"
                    style={{ "--moment": momentTone(moment, moment === waiting) }}
                  >
                    <span className="hi-tasks__moment-head">
                      <span className="hi-tasks__moment-kind">
                        {life || L.moment[moment.kind] || moment.kind}
                        {life && byHand(moment.text) ? ` \u00b7 ${L.byHand}` : ""}
                      </span>
                      {moment.at && (
                        <span className="hi-tasks__moment-at">{formatStamp(moment.at)}</span>
                      )}
                    </span>
                    {!life && (
                      <span className="hi-tasks__moment-text">
                        {inline(moment.text, `m${index}`, linkFile)}
                      </span>
                    )}
                  </li>
                );
              })}
            </ol>
          )}

          {(rest.length > 0 || task.extraDropped > 0) && (
            <details className="hi-tasks__notes hi-tasks__fields">
              <summary>{L.fields}</summary>
              <dl>
                {rest.map((field, index) => (
                  <div key={`${field.key}-${index}`}>
                    {field.key && <dt>{field.key}</dt>}
                    <dd>
                      {/* Autolinked for the reason a `waiting` line is: the panel has no
                          address bar, and a URL the agent filed under `report_to:` or a
                          dated note key is otherwise one to retype off the screen. Through
                          `linked` and not `inline` — a field value is a literal, so the
                          asterisks and backticks in it are its own characters. */}
                      {linked(
                        field.clipped ? `${field.value}\u2026` : field.value,
                        `f${index}`,
                      )}
                    </dd>
                  </div>
                ))}
              </dl>
              {task.extraDropped > 0 && (
                <div className="hi-tasks__fields-more">{L.moreFields(task.extraDropped)}</div>
              )}
            </details>
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

// A card's verb may carry a shorter label than the panel's, with the full wording kept as
// the accessible name — a column is too narrow for "Mark done", a screen reader is not.
//
// **A card carries one verb, and only where that verb is a flick**: start this, finish this.
// Three buttons on every card was a strip of chrome heavier than the sentence above it, on
// a surface whose whole job is to be scanned — and on the closed columns it was a hundred
// and thirty Reopens for a transition nobody takes. Cancelling, standing a duty down, and
// keeping something as a standing duty are *decisions*: they belong next to the record, in
// the panel, which is reachable by click, tap and Enter alike. Drag remains the shortcut it
// was meant to be rather than the only way anywhere.
function Actions({ task, busy, onStatus, card }) {
  const button = (kind, status, label, cardLabel) => (
    <button
      type="button"
      className={`hi-tasks__button hi-tasks__button--${kind}`}
      disabled={busy}
      aria-label={label}
      title={label}
      onClick={() => onStatus(task.subject, status)}
    >
      {card && cardLabel ? cardLabel : label}
    </button>
  );

  // `draggable={false}` so pressing a button never starts a drag of the card around it:
  // a draggable element otherwise hands its whole subtree to the drag.
  // Ghost, not primary. A filled button is the loudest thing it sits near, and a column of
  // five of them competes with the one card on the board that is actually asking for
  // something. The card's verb is there when it is wanted; it is not the point of the card.
  // The panel still fills its own primary, where there is one action among four.
  if (card) {
    if (task.status === "todo") {
      return (
        <div className="hi-tasks__actions" draggable={false}>
          {button("ghost", "doing", L.start)}
        </div>
      );
    }
    if (task.status === "doing") {
      return (
        <div className="hi-tasks__actions" draggable={false}>
          {button("ghost", "done", L.markDone, L.shortDone)}
        </div>
      );
    }
    // A duty has no flick: it ends by being stood down, which is a decision about something
    // that has been running for weeks. Closed rows are in the rail and carry no verb at all.
    return null;
  }

  if (task.status === "done" || task.status === "cancelled") {
    return (
      <div className="hi-tasks__actions" draggable={false}>
        {button("ghost", "todo", L.reopen)}
      </div>
    );
  }

  // A duty has no "done" to offer: it ends by being stood down, which is the same close
  // wearing the name of what actually happened.
  if (task.status === "serving") {
    return (
      <div className="hi-tasks__actions" draggable={false}>
        {button("ghost", "todo", L.moveTodo)}
        {button("danger", "cancelled", L.cancel)}
        {button("primary", "done", L.standDown)}
      </div>
    );
  }

  return (
    <div className="hi-tasks__actions" draggable={false}>
      {task.status === "todo"
        ? button("ghost", "doing", L.start)
        : button("ghost", "todo", L.moveTodo)}
      {button("ghost", "serving", L.serve)}
      {button("danger", "cancelled", L.cancel)}
      {button("primary", "done", L.markDone)}
    </div>
  );
}

// The instant a row closed, for ordering the rail. `statusSince` is the fallback because a
// record written before those stamps existed has neither, and a row with no time at all must
// not sort to the top as though it had just happened.
function closedStamp(task) {
  const at = task.completedAt || task.cancelledAt || task.statusSince || task.createdAt;
  const ms = at ? new Date(at).getTime() : NaN;
  return Number.isNaN(ms) ? 0 : ms;
}

// Day granularity, deliberately. A rail a hundred rows long is read for *when roughly*, and
// at that density the minute is noise; the panel still carries the full stamp.
function formatDay(ms) {
  if (!ms) return "";
  const date = new Date(ms);
  const sameYear = date.getFullYear() === new Date().getFullYear();
  return new Intl.DateTimeFormat(LOCALE, {
    month: "numeric",
    day: "numeric",
    ...(sameYear ? {} : { year: "2-digit" }),
  }).format(date);
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

// The newest line a *mind* wrote. `moved` is excluded because the store writes it on a
// transition it merely witnessed: a status change is the consequence of a decision, not a
// statement about one, so it can neither raise a wait nor answer it.
function latestSpoken(task) {
  const timeline = task.timeline || [];
  for (let i = timeline.length - 1; i >= 0; i -= 1) {
    if (timeline[i].kind !== "moved") return timeline[i];
  }
  return null;
}

// **Whether a human is wanted right now** — the one question the record could not answer
// before, and the reason `blocked` became `waiting`.
//
// Nothing clears a `waiting` line; it is a dated sentence in a record that only appends.
// What makes it current is that no mind has written anything under it. So a wait ends by
// being superseded rather than by being closed, which needs no second kind to say "no
// longer waiting" and cannot go stale the way a flag somebody must remember to unset does.
// A closed row is never waiting, whatever its last line says.
function waitsOnPerson(task) {
  if (task.status === "done" || task.status === "cancelled") return false;
  return latestSpoken(task)?.kind === "waiting";
}

// A status change, in the word for it. The store writes the transition as the pair it
// witnessed — `todo → doing` — and that is the right thing to *store*; it is the wrong
// thing to show somebody catching up on their own errand.
function lifecycleWord(text) {
  const parts = String(text || "").split("\u2192");
  if (parts.length < 2) return null;
  const from = parts[0].trim();
  // The store signs the transitions made from this board — `done (on the board)` — so a
  // reader can tell their own close from a rung's. The mark is the tail of the line and not
  // the status, so it comes off before the word is looked up; `byHand` shows it separately.
  const to = parts[parts.length - 1].replace(/\s*\(on the board\)\s*$/, "").trim();
  if (to === "todo") {
    return from === "done" || from === "cancelled" ? L.life.reopened : L.life.putBack;
  }
  return (
    { doing: L.life.started, serving: L.life.serving, done: L.life.done, cancelled: L.life.cancelled }[
      to
    ] || null
  );
}

// Whether the store signed this transition as the reader's own. Shown out loud, because a
// bare verb reads as the agent's doing and some of these were done by the person reading.
function byHand(text) {
  return /\(on the board\)\s*$/.test(String(text || ""));
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
function elapsed(value) {
  const then = new Date(value).getTime();
  if (Number.isNaN(then)) return null;
  const mins = Math.max(0, Math.round((Date.now() - then) / 60000));
  if (mins < 60) return { n: mins, unit: L.agoUnits.m, hours: mins / 60 };
  if (mins < 60 * 24) return { n: Math.floor(mins / 60), unit: L.agoUnits.h, hours: mins / 60 };
  return { n: Math.floor(mins / (60 * 24)), unit: L.agoUnits.d, hours: mins / 60 };
}

// How long work may sit in `doing` before its age stops being a fact to note and becomes one
// to answer. The same 48 hours `tasks::IDLE_BOUNDARY_HOURS` reads, deliberately duplicated
// rather than served: it decides a colour here and a whole sentence there, and a board that
// fetched it would still have to decide what to do with it.
const IDLE_BOUNDARY_HOURS = 48;

// How long this row has stood where it stands.
//
// **`todo` and `doing` only.** A duty is *supposed* to be old — its age says nothing and
// "last confirmed alive" says everything — and a closed row already carries the stamp of its
// closing. The one status that promises an ending is the one that can fail to reach it.
//
// Below a day the number is noise on a surface read many times a day, so nothing is said and
// the creation date keeps the space. Past the idle boundary it is the fact on the card.
function ageMeta(task) {
  if (task.status !== "todo" && task.status !== "doing") return null;
  // The store's own fallback for a record that never recorded a transition: older than the
  // truth, so it errs toward the boundary rather than hiding behind it.
  const at = task.statusSince || task.createdAt;
  if (!at) return null;
  const span = elapsed(at);
  if (!span || span.hours < 24) return null;
  return {
    text: L.standing(L.category[task.status], `${span.n}${span.unit}`),
    warn: task.status === "doing" && span.hours >= IDLE_BOUNDARY_HOURS,
  };
}

// The switchboard folded to one session per subject, because a task can have more than one
// registered under it and the card has room for one answer. The most alive wins: running over
// idle, and among equals the one that moved most recently. Reporting the quietest of two
// sessions would turn a task that *is* being worked into an alarm.
function bySubject(workers) {
  const rank = (worker) => (worker.state === "running" ? 2 : 1);
  const map = new Map();
  for (const worker of workers) {
    if (!worker.subject) continue;
    const held = map.get(worker.subject);
    if (
      !held ||
      rank(worker) > rank(held) ||
      (rank(worker) === rank(held) &&
        new Date(worker.state_since || 0) > new Date(held.state_since || 0))
    ) {
      map.set(worker.subject, worker);
    }
  }
  return map;
}

// Whether anybody is on this row right now — the question `doing` raises and the record
// cannot answer.
//
// **A `doing` row with no live session looks exactly like one being worked.** The card shows a
// title, a date and the last thing written, all of which a task whose worker died mid-turn
// keeps unchanged forever. On one live ledger 9 of 11 `doing` rows had nobody running on them,
// drawn identically to the one that did.
//
// **Which absences are worth saying is not the same question for every status**, and this
// follows the rule `tasks::worker_note` already reasoned out on the agent's side: a live worker
// is reported wherever there is one, because that is positive information and cannot be a false
// alarm — but *nobody* is said only on `doing`, where nobody is the alarm. A `todo` with no
// worker is what a `todo` is, and a duty spends most of its life between bursts; printing it
// there would put the phrase on most of the board and teach the eye to skip it, and then it
// would be skipped on the one row where it means something.
//
// `undefined` is the third answer and is not "nobody": until the first roster lands, nothing is
// known and nothing is said.
//
// **`idle` on a `doing` row is a warning, not a state.** It is the same word a worker waiting
// for its next instruction reports, so the row that should read as stalled reads as patient —
// but on `doing` the turn ended and nothing moved the row, which is the stall itself.
function onItMeta(task) {
  const worker = task.onIt;
  if (worker === undefined) return null;
  if (!worker) return task.status === "doing" ? { text: L.nobodyOnIt, warn: true } : null;
  const span = elapsed(worker.state_since);
  const stamp = span ? `${span.n}${span.unit}` : "";
  const running = worker.state === "running";
  return {
    text: running ? L.onItRunning(stamp) : L.onItIdle(stamp),
    warn: task.status === "doing" && !running,
  };
}

// The switchboard's line as the newest entry of the record — in the panel, which is the one
// place with room to print it whole.
//
// **This is not a seventh stored kind and must never read as one.** Nothing appends it, nothing
// keeps it, and it changes on every poll; headed with a wall clock at the top of an append-only
// record it would make a row that has not moved in half an hour read as one that just did. So
// it carries no instant, only an age — *2m ago* — which is the one phrasing that cannot be
// mistaken for something written down.
//
// **That age is the line's own clock and not the worker's**, which is what stops it repeating
// the status chip two inches above it. The chip answers how long this session has been running;
// `doing_at` answers when this step last changed, and the gap between those two numbers is the
// difference between a worker working and one hung — the thing neither field says alone.
//
// What it prints is what used to hang off the status chip, clipped to 72 characters with its
// newlines collapsed. The line arrives already bounded — `registry::record_activity` cuts it at
// `ACTIVITY_LINE_CHARS` (120), and a turn's `error` at `OUTCOME_LINE_CHARS` — so the 72 was the
// chip's own limit and not the wire's: a shell command reached this panel with its second half
// gone, which is the half naming the file. Here it wraps like every other entry, and the status
// chip goes back to the one fact it can hold — alive or not, and for how long.
//
// The running/quiet split is `onItMeta`'s and holds for its reason: while a worker is busy,
// what it is doing now is the answer and last turn's ending is stale. Never both.
function liveMoment(task) {
  const worker = task.onIt;
  if (!worker) return null;
  const running = worker.state === "running";
  const said = ((running ? worker.doing : worker.last_turn?.error) || "").trim();
  if (!said) return null;
  const at = running ? worker.doing_at : worker.last_turn?.at;
  const span = at ? elapsed(at) : null;
  return {
    kind: running ? "live" : "failed",
    text: said,
    // Read by the panel in place of `at`, which stays absent on purpose.
    ago: span ? L.ago(`${span.n}${span.unit}`) : "",
  };
}

// Every duty gets this line, including one with no `liveness` recorded — a duty nobody
// wrote a check for is the worse case, not an exempt one, and silence there reads as fine.
function healthMeta(task) {
  if (task.status !== "serving") return null;
  if (!task.checkedAt) return { text: L.neverChecked, warn: true };
  return { text: L.checked(formatStamp(task.checkedAt)), warn: false };
}

function taskNeedsAttention(task) {
  return Boolean(
    waitsOnPerson(task) || dueMeta(task)?.warn || healthMeta(task)?.warn || ageMeta(task)?.warn,
  );
}

// **The one line the card gets** — the single fact that would make someone act on this row,
// or reassure them that nobody has to. It used to be a stack: a warning, then a date, on top
// of a clipped record line above it, which is three lines of small type per card and no
// order to read them in.
//
// The order below is the priority, and it is a priority precisely because only the first
// survives. A warning outranks everything: on a `doing` row the record is exactly what a
// task whose worker died keeps looking fine by, so the dead worker is the line. With nothing
// wrong, the reassuring half of that same question takes the space — this one is alive —
// and only then a date.
//
// A wait never reaches here; the card renders that itself, with the sentence, because
// "somebody is waiting on you" without saying what for is an alarm with no verb attached.
function cardNote(task) {
  if (task.malformed) return { text: L.malformedShort, warn: true };
  const on = onItMeta(task);
  const age = ageMeta(task);
  const due = dueMeta(task);
  const health = healthMeta(task);
  for (const note of [on, age, due, health]) {
    if (note?.warn) return note;
  }
  // Never "Created Aug 3" on a duty: a watch is supposed to be old, so its age is the one
  // fact about it that means nothing. Whether it is still up is the only one that does.
  if (task.status === "serving") return health || on || null;
  if (on) return on;
  if (due) return due;
  // "Created Aug 3" and "doing for 6d" are the same row's story, and only the second says it
  // has been sitting.
  if (age) return age;
  if (task.createdAt) return { text: L.created(formatStamp(task.createdAt)) };
  return null;
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
// And a URL is exactly what a `waiting` row has to carry — the panel has no
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
//
// `link` is the other half of that: optional, and it resolves a code span to a file the task
// actually has, so a name the record wrote becomes the file it names. It is given the token
// verbatim and answers `null` for everything else, which is nearly everything — these bodies
// spell `hi_say`, `status_since` and a SHA-256 the same way they spell a filename.
function inline(text, keyBase, link) {
  const out = [];
  let i = 0;
  for (const part of String(text).split(INLINE)) {
    if (!part) continue;
    const key = `${keyBase}-${i++}`;
    if (part.length > 4 && part.startsWith("**") && part.endsWith("**")) {
      out.push(<strong key={key}>{linked(part.slice(2, -2), key)}</strong>);
    } else if (part.length > 2 && part.startsWith("`") && part.endsWith("`")) {
      const token = part.slice(1, -1);
      const href = link ? link(token) : null;
      out.push(
        href ? (
          <a key={key} className="hi-tasks__file" href={href} target="_blank" rel="noreferrer">
            <code>{token}</code>
          </a>
        ) : (
          <code key={key}>{token}</code>
        ),
      );
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

function Prose({ text, link }) {
  return (
    <div className="hi-tasks__prose">
      {blocks(text).map((block, i) => {
        if (block.kind === "h") {
          return (
            <div key={i} className="hi-tasks__prose-h" data-depth={Math.min(block.depth, 3)}>
              {inline(block.text, i, link)}
            </div>
          );
        }
        if (block.kind === "ul") {
          return (
            <ul key={i} className="hi-tasks__prose-ul">
              {block.items.map((item, j) => (
                <li key={j}>{inline(item, `${i}-${j}`, link)}</li>
              ))}
            </ul>
          );
        }
        return (
          <p key={i} className="hi-tasks__prose-p">
            {inline(block.text, i, link)}
          </p>
        );
      })}
    </div>
  );
}

// The account, in the panel rather than behind a fold.
//
// It was a `<details>` under the timeline, and what that hid was the answer: a report went
// back in conversation, the row closed, and the one paragraph saying what was found — 90%
// used, this writer, this much a day — sat collapsed below a reverse-chronological list of
// housekeeping. Somebody coming back to their own errand a week later opened the panel and
// saw that it had been updated, not what it said. A fold is the right shape for a
// reference; it is the wrong shape for the thing the reader came for.
//
// **Clamped, not truncated.** The reason for the fold was real — the live store's median
// body is 3.2 KB and its largest 48 KB, and unfolding that on top of the timeline buries
// it just as thoroughly. So the first screenful reads by itself and the rest is one click
// away, and the button only exists when there is something under it: measured, because a
// guess from character count is wrong in both directions on prose this varied.
function Account({ text, link, label }) {
  const [expanded, setExpanded] = useState(false);
  const [overflows, setOverflows] = useState(false);
  const box = useRef(null);

  // The clamp is unconditional while collapsed, and the measurement only decides whether
  // there is a button. Hanging the clamp itself on the measurement is the bug this
  // comment exists to stop coming back: unclamped, `scrollHeight` equals `clientHeight`,
  // so nothing ever reads as too long and a 48 KB account renders whole.
  useLayoutEffect(() => {
    if (expanded) return;
    const el = box.current;
    if (el) setOverflows(el.scrollHeight - el.clientHeight > 4);
  }, [text, expanded]);

  return (
    <div className="hi-tasks__account">
      <div className="hi-tasks__moments-title">{label}</div>
      <div
        ref={box}
        className="hi-tasks__account-body"
        data-clamped={expanded ? undefined : "true"}
        data-fade={!expanded && overflows ? "true" : undefined}
      >
        <Prose text={text} link={link} />
      </div>
      {overflows && (
        <button
          type="button"
          className="hi-tasks__more"
          onClick={() => setExpanded((was) => !was)}
        >
          {expanded ? L.showLess : L.showAll}
        </button>
      )}
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

  .hi-tasks__heading span {
    color: var(--fg-dim);
    font-size: 13px;
    font-weight: 650;
    font-variant-numeric: tabular-nums;
  }

  .hi-tasks__heading .hi-tasks__heading-serving {
    color: var(--task-serving);
  }

  /* The only count that is somebody else's move, so it is the only one that gets a
     ground of its own. Zero of these renders nothing at all — a badge that is usually
     empty is worth more than one that is usually a nought. */
  .hi-tasks__heading .hi-tasks__heading-needs {
    padding: 3px 9px;
    border-radius: 999px;
    background: var(--danger-wash);
    color: var(--danger);
    font-weight: 780;
  }

  /* The board is the page: three columns of live work and the ledger rail, filling the
     height and never scrolling it. The rail is deliberately the narrow one — a row in it is
     one line, and the three that hold what is still owed get the rest. Below ~1000px the
     tracks keep a readable floor and the board scrolls sideways rather than squeezing every
     card into a ribbon; below 760px it stops being a board at all (see the tail of this
     sheet). */
  .hi-tasks__board {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: repeat(3, minmax(236px, 1fr)) minmax(228px, 0.8fr);
    gap: 12px;
    /* A board is the case the bottom token exists for: a card half under a control
       disc is a lost row, not texture passing behind glass. */
    padding: 14px clamp(16px, 2.4vw, 30px) calc(16px + var(--hi-chrome-bottom));
    overflow-x: auto;
    overflow-y: hidden;
  }

  .hi-tasks__column,
  .hi-tasks__ledger {
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
  .hi-tasks__column[data-drop="true"],
  .hi-tasks__ledger[data-drop="true"] {
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

  /* The floating dock sits over the bottom-right corner of the surface, which is the rail's
     tail now that the rail is the last track. Only it pays for the clearance. */
  .hi-tasks__ledger .hi-tasks__ledger-rows {
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

  /* A row that will not move until the person does. It sorts to the top of its column and
     it is the one card on the board wearing a ground — the alarm was a line of red 11px type
     in the fourth card down, which is a thing you find rather than a thing you see. */
  .hi-tasks__card[data-needs="true"] {
    border-color: var(--danger-line);
    border-left-color: var(--danger);
    background: var(--danger-wash);
  }

  .hi-tasks__card[data-needs="true"]:hover {
    border-top-color: var(--danger-line);
    border-right-color: var(--danger-line);
    border-bottom-color: var(--danger-line);
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

  /* The wait: the flag, then as much of the sentence as fits on one line. Two lines,
     because this is the one thing on the board somebody has to read rather than glance at,
     and a single ellipsised line of it is a demand with the reason cut off. */
  .hi-tasks__card-wait {
    display: flex;
    align-items: baseline;
    gap: 6px;
    margin-top: 6px;
    min-width: 0;
  }

  .hi-tasks__card-wait-flag {
    flex: none;
    color: var(--danger);
    font-size: 11.5px;
    line-height: 1.4;
    font-weight: 800;
  }

  .hi-tasks__card-wait-text {
    flex: 1;
    min-width: 0;
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    overflow: hidden;
    color: var(--fg-dim);
    font-size: 11.5px;
    line-height: 1.4;
    font-weight: 620;
    overflow-wrap: anywhere;
  }

  .hi-tasks__card-note {
    margin-top: 6px;
    overflow: hidden;
    color: var(--fg-dim);
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 11.5px;
    line-height: 1.4;
    font-weight: 620;
    font-variant-numeric: tabular-nums;
  }

  .hi-tasks__card-note[data-warn="true"] {
    color: var(--danger);
  }

  /* A closed row is a line, not a card: no border, no ground, no verb — the hover is the
     only chrome it gets, and it is there to say the row opens. Three columns of grid so the
     dates line up down the right edge and the eye can read the rail as the chronology it
     is, without the dates jittering behind titles of different lengths. */
  .hi-tasks__ledger-rows {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 6px;
    overflow-y: auto;
  }

  .hi-tasks__ledger-row {
    flex: none;
    display: grid;
    grid-template-columns: 6px minmax(0, 1fr) auto;
    align-items: center;
    gap: 9px;
    width: 100%;
    padding: 6px 8px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    text-align: left;
    transition: background 160ms var(--ease);
  }

  .hi-tasks__ledger-row:hover {
    background: var(--surface);
  }

  /* Which of the two closes it was, in the smallest mark that can carry it: sage for
     finished, a muted red for cancelled. It was a column and a word on every card. */
  .hi-tasks__ledger-mark {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent-2);
  }

  .hi-tasks__ledger-row[data-status="cancelled"] .hi-tasks__ledger-mark {
    background: var(--danger);
    opacity: 0.5;
  }

  .hi-tasks__ledger-title {
    min-width: 0;
    overflow: hidden;
    color: var(--fg-dim);
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 12.5px;
    line-height: 1.45;
    font-weight: 680;
  }

  .hi-tasks__ledger-row[data-status="cancelled"] .hi-tasks__ledger-title {
    color: var(--fg-mute);
  }

  .hi-tasks__ledger-when {
    color: var(--fg-mute);
    font-size: 11px;
    line-height: 1.45;
    font-weight: 650;
    font-variant-numeric: tabular-nums;
  }

  /* Only while a card is in the air, because that is the only moment the rail is a target
     and the only moment the gesture needs naming. */
  .hi-tasks__ledger-hint {
    margin-left: auto;
    padding-right: 2px;
    color: var(--accent);
    font-size: 11px;
    line-height: 1;
    font-weight: 750;
    white-space: nowrap;
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

  /* What the record says this touches, read as tags rather than as a field — it is the
     answer to "what is this about", and it belongs beside the title. */
  .hi-tasks__systems {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
    margin-bottom: 14px;
  }

  .hi-tasks__system {
    padding: 2px 8px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--fg-mute) 14%, transparent);
    color: var(--fg-dim);
    font-size: 11.5px;
    font-weight: 650;
  }

  /* Everything else the frontmatter carries, in the file's order and folded away: a real
     record's own ledger runs to tens of keys, and it is a thing to go and read rather than a
     thing to be shown. */
  .hi-tasks__fields dl {
    margin: 0;
    display: grid;
    grid-template-columns: minmax(0, auto) minmax(0, 1fr);
    gap: 2px 12px;
    font-size: 12.5px;
    line-height: 1.55;
  }

  .hi-tasks__fields dl > div {
    display: contents;
  }

  .hi-tasks__fields dt {
    color: var(--fg-dim);
    font-weight: 650;
    overflow-wrap: anywhere;
  }

  .hi-tasks__fields dd {
    grid-column: 2;
    margin: 0;
    color: var(--fg);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .hi-tasks__fields-more {
    margin-top: 8px;
    color: var(--fg-dim);
    font-size: 11.5px;
  }

  /* Pinned above the record and never scrolled past: one or three lines in their own
     words, so it can be read as prose rather than parsed as a field. */
  /* Louder than the pin below it, because it is the one block on this surface that is
     asking the reader for something rather than telling them how the work went. */
  .hi-tasks__waiting {
    margin-bottom: 16px;
    padding: 11px 13px;
    border-left: 3px solid var(--danger);
    background: color-mix(in srgb, var(--danger) 10%, transparent);
  }

  .hi-tasks__waiting-title {
    margin-bottom: 4px;
    color: var(--danger);
    font-size: 11.5px;
    font-weight: 750;
  }

  .hi-tasks__waiting-text {
    color: var(--fg);
    font-size: 13.5px;
    line-height: 1.6;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

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

  /* The one entry nobody wrote, so it is the one entry drawn at full strength: a solid
     stripe against every stored line's 55%, and a dot on the kind word. The dot is what
     carries "this is a reading, not a record" at a glance. */
  .hi-tasks__moment[data-live] {
    border-left-color: var(--moment);
  }

  .hi-tasks__moment[data-live] .hi-tasks__moment-kind::before {
    content: "";
    display: inline-block;
    width: 5px;
    height: 5px;
    margin-right: 5px;
    border-radius: 50%;
    background: var(--moment);
    vertical-align: middle;
  }

  /* Only the running one breathes, and slowly enough to be noticed only when looked at.
     A dead worker's last error is as still as anything else on the list — a pulse there
     would say the opposite of what the line says. */
  .hi-tasks__moment[data-live="live"] .hi-tasks__moment-kind::before {
    animation: hi-tasks-live 2.4s ease-in-out infinite;
  }

  @keyframes hi-tasks-live {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.3; }
  }

  @media (prefers-reduced-motion: reduce) {
    .hi-tasks__moment[data-live="live"] .hi-tasks__moment-kind::before {
      animation: none;
    }
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

  /* The account sits above the timeline and reads without being asked for. Clamped to a
     screenful, with the rest one click down: whole, but never the wall the fold was
     hiding. The fade is drawn on the box, so it lands on whatever the last visible line
     happens to be. */
  .hi-tasks__account {
    margin-bottom: 18px;
  }

  .hi-tasks__account-body {
    position: relative;
    max-height: none;
    overflow: visible;
  }

  .hi-tasks__account-body[data-clamped="true"] {
    /* A screenful of account, and the timeline still on screen under it. Taller and the
       spine goes back below the fold, which is the shape this was fixing. */
    max-height: 22em;
    overflow: hidden;
  }

  .hi-tasks__account-body[data-fade="true"]::after {
    content: "";
    position: absolute;
    inset: auto 0 0;
    height: 4.5em;
    background: linear-gradient(to bottom, transparent, var(--bg-0));
    pointer-events: none;
  }

  .hi-tasks__more {
    margin-top: 6px;
    padding: 0;
    border: 0;
    background: none;
    cursor: pointer;
    color: var(--accent-2);
    font: inherit;
    font-size: 11.5px;
    font-weight: 750;
  }

  .hi-tasks__more:focus-visible {
    outline: 3px solid var(--accent-soft);
    outline-offset: 2px;
  }

  /* A name the record wrote that turned out to be a file it has. It stays typographically
     the code span it was — the sentence is unchanged — and only gains the underline that
     says it opens. */
  .hi-tasks__file {
    color: inherit;
    text-decoration: underline;
    text-decoration-color: var(--accent-2);
    text-underline-offset: 2px;
  }

  .hi-tasks__file:hover code {
    color: var(--accent-2);
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
    grid-template-columns: repeat(3, minmax(236px, 1fr)) minmax(228px, 0.8fr);
    gap: 12px;
    padding: 14px clamp(16px, 2.4vw, 30px) 16px;
    overflow: hidden;
  }

  .hi-tasks__loading span {
    border: 1px solid var(--line);
    border-radius: 10px;
    background: color-mix(in srgb, var(--surface) 42%, var(--bg-0));
  }

  /* **A phone is not a small desktop.** Columns side by side on a 390px screen is a board
     you read by dragging sideways one and a half columns at a time — which is the one thing
     a board exists to save you from, and it hides most of the ledger behind a gesture
     nothing on screen suggests. Below 760px the tracks stack and the page scrolls
     vertically as one: the lifecycle is still the order, now top to bottom, and nothing is
     behind a filter. Each section keeps its head, its count and its own empty line, so an
     empty Todo still reads as answered rather than absent.
     The rail is the reason this stack is now finishable at all. Stacked as cards, a
     hundred and thirty closed rows were a hundred and thirty card-heights below the live
     work; as lines they are a scroll rather than an expedition, and they are last. */
  @media (max-width: 760px) {
    .hi-tasks__header {
      min-height: 56px;
      /* Still max(…, var(--hi-safe-top)): a notch is a phone's problem, so the
         phone breakpoint is the last place the inset may be dropped. Flattening
         this to a plain 12px is what put the heading under the status bar. */
      padding: max(12px, var(--hi-safe-top)) 16px 12px;
    }

    .hi-tasks__heading {
      gap: 9px;
    }

    .hi-tasks__heading h1 {
      font-size: 21px;
    }

    .hi-tasks__board,
    .hi-tasks__loading {
      display: flex;
      flex-direction: column;
      gap: 10px;
      padding: 12px 14px calc(14px + var(--hi-chrome-bottom));
      overflow-x: hidden;
      overflow-y: auto;
    }

    /* A column becomes a section: it grows to its cards and the board is the
       one scroller, rather than four boxes each owning a scroller inside a
       screen with room for none of them. */
    .hi-tasks__column,
    .hi-tasks__ledger,
    .hi-tasks__cards,
    .hi-tasks__ledger-rows {
      flex: none;
      overflow: visible;
    }

    /* The dock's clearance moved to the board's own tail above, so the rail no
       longer pays for it — stacked, it is simply the last section. */
    .hi-tasks__ledger .hi-tasks__ledger-rows {
      padding-bottom: 10px;
    }

    .hi-tasks__loading span {
      flex: none;
      height: 92px;
    }
  }
`;
