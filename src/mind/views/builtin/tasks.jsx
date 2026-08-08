// Built-in task ledger. The full canvas is organized by the one durable lifecycle:
// todo, doing, done, cancelled. Liveness is optional detail on a doing task, never a
// second kind or mode.
import { useState, useEffect, useCallback } from "react";

export const captionAside = "top";

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
    status: "Status",
    statusNav: "Task status",
    category: {
      todo: "Todo",
      doing: "Doing",
      done: "Done",
      cancelled: "Cancelled",
    },
    empty: {
      todo: "No tasks waiting to start.",
      doing: "Nothing is being worked on.",
      done: "No completed tasks yet.",
      cancelled: "No cancelled tasks.",
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
    malformed: "This task has invalid stored fields. Changing its status will rewrite the recognized fields.",
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
    status: "状态",
    statusNav: "任务状态",
    category: {
      todo: "待办",
      doing: "进行中",
      done: "已完成",
      cancelled: "已取消",
    },
    empty: {
      todo: "没有等待开始的任务。",
      doing: "目前没有正在处理的任务。",
      done: "还没有已完成的任务。",
      cancelled: "没有已取消的任务。",
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
    malformed: "这条任务包含无效字段。修改状态时会重写可识别的字段。",
    noBody: "（没有备注）",
    monitoring: "运行检查",
    verify: "检查方式",
    restart: "停止后",
    owner: "负责人",
    startKey: "启动标识",
  },
};

function words() {
  const app = document.documentElement.lang || "";
  const chain = !app || /^system$/i.test(app) ? [navigator.language] : [app, navigator.language];
  for (const tag of chain) {
    if (/^zh\b/i.test(tag || "")) return T.zh;
    if (/^en\b/i.test(tag || "")) return T.en;
  }
  return T.en;
}
const L = words();

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
  const [status, setStatus] = useState("doing");
  const [busy, setBusy] = useState(null);

  const reload = useCallback(async () => {
    const data = await api.list().catch(() => ({ tasks: [] }));
    setTasks(data.tasks || []);
  }, []);

  useEffect(() => {
    reload();
  }, [reload]);

  const setTaskStatus = async (subject, nextStatus) => {
    setBusy(subject);
    await api.patch(subject, { status: nextStatus }).catch(() => {});
    setBusy(null);
    reload();
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
        </div>
      </div>
    );
  }

  const activeCount = tasks.filter((task) => task.status === "todo" || task.status === "doing").length;
  const visible = tasks.filter((task) => task.status === status);
  const selected = STATUSES.find((item) => item.id === status) || STATUSES[1];
  const attention = visible.filter(taskNeedsAttention).length;

  return (
    <div className="hi-tasks">
      <style>{CSS}</style>
      <Header activeCount={activeCount} totalCount={tasks.length} />

      <div className="hi-tasks__workspace">
        <nav className="hi-tasks__statuses" aria-label={L.statusNav}>
          <div className="hi-tasks__statuses-title">{L.status}</div>
          <div className="hi-tasks__status-list">
            {STATUSES.map((item) => {
              const count = tasks.filter((task) => task.status === item.id).length;
              return (
                <button
                  key={item.id}
                  type="button"
                  className="hi-tasks__status"
                  data-active={item.id === status ? "true" : undefined}
                  data-tone={item.tone}
                  aria-pressed={item.id === status}
                  onClick={() => setStatus(item.id)}
                >
                  <span className="hi-tasks__status-dot" aria-hidden="true" />
                  <span className="hi-tasks__status-label">{item.label}</span>
                  <span className="hi-tasks__status-count">{count}</span>
                </button>
              );
            })}
          </div>
        </nav>

        <main className="hi-tasks__content">
          <div className="hi-tasks__content-head">
            <div>
              <h2>{selected.label}</h2>
              <div className="hi-tasks__content-count">{L.totalN(visible.length)}</div>
            </div>
            {attention > 0 && <div className="hi-tasks__attention">{L.attentionN(attention)}</div>}
          </div>

          {visible.length === 0 ? (
            <div className="hi-tasks__empty">{L.empty[status]}</div>
          ) : (
            <div className="hi-tasks__list">
              {visible.map((task) => (
                <Row
                  key={task.subject}
                  task={task}
                  busy={busy === task.subject}
                  onStatus={setTaskStatus}
                />
              ))}
            </div>
          )}
        </main>
      </div>
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

function Row({ task, busy, onStatus }) {
  const [expanded, setExpanded] = useState(false);
  const tone = STATUS_TONE[task.status] || "var(--fg-mute)";
  const due = dueMeta(task);
  const health = healthMeta(task);
  const closedAt =
    task.status === "done" && task.completedAt
      ? L.completed(formatStamp(task.completedAt))
      : task.status === "cancelled" && task.cancelledAt
        ? L.cancelled(formatStamp(task.cancelledAt))
        : null;

  return (
    <article
      className="hi-tasks__row"
      data-closed={task.status === "done" || task.status === "cancelled" ? "true" : undefined}
      data-malformed={task.malformed ? "true" : undefined}
      aria-busy={busy}
      style={{ "--task-status": tone }}
    >
      <button
        type="button"
        className="hi-tasks__row-toggle"
        aria-expanded={expanded}
        onClick={() => setExpanded((value) => !value)}
      >
        <span className="hi-tasks__status-bar" aria-hidden="true" />
        <span className="hi-tasks__row-copy">
          <span className="hi-tasks__row-title">{task.title || task.subject}</span>
          <span className="hi-tasks__row-meta">
            <span className="hi-tasks__status-chip">{L.category[task.status] || task.status}</span>
            {task.createdAt && <span>{L.created(formatStamp(task.createdAt))}</span>}
            {closedAt && <span>{closedAt}</span>}
            {due && <span data-warn={due.warn ? "true" : undefined}>{due.text}</span>}
            {health && <span data-warn={health.warn ? "true" : undefined}>{health.text}</span>}
          </span>
        </span>
        <span className="hi-tasks__chevron" data-open={expanded ? "true" : undefined} aria-hidden="true" />
      </button>

      <Actions task={task} busy={busy} onStatus={onStatus} />

      {task.malformed && <div className="hi-tasks__bad">{L.malformed}</div>}

      {expanded && (
        <div className="hi-tasks__body">
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
      )}
    </article>
  );
}

function Actions({ task, busy, onStatus }) {
  if (task.status === "done" || task.status === "cancelled") {
    return (
      <div className="hi-tasks__actions">
        <button
          type="button"
          className="hi-tasks__button hi-tasks__button--ghost"
          disabled={busy}
          onClick={() => onStatus(task.subject, "todo")}
        >
          {L.reopen}
        </button>
      </div>
    );
  }

  return (
    <div className="hi-tasks__actions">
      {task.status === "todo" ? (
        <button
          type="button"
          className="hi-tasks__button hi-tasks__button--ghost"
          disabled={busy}
          onClick={() => onStatus(task.subject, "doing")}
        >
          {L.start}
        </button>
      ) : (
        <button
          type="button"
          className="hi-tasks__button hi-tasks__button--ghost"
          disabled={busy}
          onClick={() => onStatus(task.subject, "todo")}
        >
          {L.moveTodo}
        </button>
      )}
      <button
        type="button"
        className="hi-tasks__button hi-tasks__button--danger"
        disabled={busy}
        onClick={() => onStatus(task.subject, "cancelled")}
      >
        {L.cancel}
      </button>
      <button
        type="button"
        className="hi-tasks__button hi-tasks__button--primary"
        disabled={busy}
        onClick={() => onStatus(task.subject, "done")}
      >
        {L.markDone}
      </button>
    </div>
  );
}

function formatStamp(value) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  const sameYear = date.getFullYear() === new Date().getFullYear();
  return new Intl.DateTimeFormat(undefined, {
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

const CSS = `
  .hi-tasks,
  .hi-tasks * {
    box-sizing: border-box;
  }

  .hi-tasks {
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
    min-height: 82px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 20px;
    padding: 18px clamp(22px, 3vw, 44px);
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
    font-size: 28px;
    line-height: 1.1;
    font-weight: 800;
    letter-spacing: 0;
  }

  .hi-tasks__heading span,
  .hi-tasks__total,
  .hi-tasks__content-count {
    color: var(--fg-dim);
    font-size: 13px;
    font-weight: 650;
    font-variant-numeric: tabular-nums;
  }

  .hi-tasks__workspace {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(190px, 230px) minmax(0, 1fr);
  }

  .hi-tasks__statuses {
    min-height: 0;
    padding: 24px 14px 116px clamp(18px, 2.5vw, 34px);
    overflow-y: auto;
    border-right: 1px solid var(--line);
    background: color-mix(in srgb, var(--surface) 48%, var(--bg-0));
  }

  .hi-tasks__statuses-title {
    margin: 0 10px 10px;
    color: var(--fg-dim);
    font-size: 12px;
    line-height: 1.4;
    font-weight: 750;
  }

  .hi-tasks__status-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .hi-tasks__status {
    --status-tone: var(--fg-mute);
    width: 100%;
    min-height: 44px;
    display: grid;
    grid-template-columns: 8px minmax(0, 1fr) auto;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border: 1px solid transparent;
    border-radius: 7px;
    text-align: left;
    color: var(--fg-dim);
    transition: background 160ms var(--ease), border-color 160ms var(--ease), color 160ms var(--ease);
  }

  .hi-tasks__status[data-tone="accent"] { --status-tone: var(--accent); }
  .hi-tasks__status[data-tone="secondary"] { --status-tone: var(--accent-2); }
  .hi-tasks__status[data-tone="danger"] { --status-tone: var(--danger); }

  .hi-tasks__status:hover {
    background: var(--surface);
    color: var(--fg);
  }

  .hi-tasks__status[data-active="true"] {
    border-color: var(--line);
    background: var(--surface-strong);
    color: var(--fg);
  }

  .hi-tasks__status-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--status-tone);
  }

  .hi-tasks__status-label {
    min-width: 0;
    overflow-wrap: anywhere;
    font-size: 13.5px;
    line-height: 1.35;
    font-weight: 680;
  }

  .hi-tasks__status-count {
    min-width: 24px;
    height: 24px;
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

  .hi-tasks__content {
    min-width: 0;
    min-height: 0;
    overflow-y: auto;
    padding-bottom: 120px;
  }

  .hi-tasks__content-head {
    position: sticky;
    top: 0;
    z-index: 2;
    min-height: 76px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 20px;
    padding: 16px clamp(20px, 3vw, 40px);
    border-bottom: 1px solid var(--line);
    background: color-mix(in srgb, var(--bg-0) 94%, transparent);
    backdrop-filter: blur(12px);
    -webkit-backdrop-filter: blur(12px);
  }

  .hi-tasks__content-head h2 {
    margin: 0 0 3px;
    color: var(--fg);
    font-size: 16px;
    line-height: 1.3;
    font-weight: 780;
    letter-spacing: 0;
  }

  .hi-tasks__attention {
    color: var(--danger);
    font-size: 12.5px;
    line-height: 1.4;
    font-weight: 750;
  }

  .hi-tasks__list {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 18px clamp(20px, 3vw, 40px) 0;
  }

  .hi-tasks__row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    grid-template-areas:
      "toggle actions"
      "bad bad"
      "body body";
    align-items: start;
    overflow: hidden;
    border: 1px solid var(--line);
    border-radius: 8px;
    background: var(--surface);
    transition: background 160ms var(--ease), border-color 160ms var(--ease);
  }

  .hi-tasks__row:hover {
    border-color: var(--line-strong);
    background: var(--surface-strong);
  }

  .hi-tasks__row[data-malformed="true"] {
    border-color: var(--danger-line);
  }

  .hi-tasks__row[data-closed="true"] {
    background: transparent;
  }

  .hi-tasks__row-toggle {
    grid-area: toggle;
    width: 100%;
    min-width: 0;
    min-height: 76px;
    display: grid;
    grid-template-columns: 4px minmax(0, 1fr) 18px;
    align-items: center;
    gap: 14px;
    padding: 13px 10px 13px 14px;
    text-align: left;
  }

  .hi-tasks__status-bar {
    width: 4px;
    height: 38px;
    border-radius: 2px;
    background: var(--task-status);
  }

  .hi-tasks__row-copy {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 7px;
  }

  .hi-tasks__row-title {
    min-width: 0;
    color: var(--fg);
    font-size: 15px;
    line-height: 1.42;
    font-weight: 680;
    overflow-wrap: anywhere;
  }

  .hi-tasks__row-meta {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 7px 12px;
    color: var(--fg-dim);
    font-size: 12px;
    line-height: 1.35;
    font-weight: 620;
  }

  .hi-tasks__row-meta [data-warn="true"] {
    color: var(--danger);
  }

  .hi-tasks__status-chip {
    padding: 3px 7px;
    border-radius: 5px;
    background: color-mix(in srgb, var(--task-status) 10%, transparent);
    color: var(--task-status);
    font-weight: 750;
  }

  .hi-tasks__row-meta span {
    white-space: nowrap;
  }

  .hi-tasks__chevron {
    width: 8px;
    height: 8px;
    border-right: 1.5px solid var(--fg-dim);
    border-bottom: 1.5px solid var(--fg-dim);
    transform: rotate(45deg) translate(-2px, -2px);
    transition: transform 160ms var(--ease);
  }

  .hi-tasks__chevron[data-open="true"] {
    transform: rotate(225deg) translate(-1px, -1px);
  }

  .hi-tasks__actions {
    grid-area: actions;
    align-self: center;
    display: flex;
    gap: 8px;
    padding: 12px 12px 12px 6px;
  }

  .hi-tasks__button {
    min-height: 44px;
    padding: 0 13px;
    border-radius: 6px;
    font-size: 13px;
    font-weight: 750;
    white-space: nowrap;
    transition: background 160ms var(--ease), border-color 160ms var(--ease), opacity 160ms var(--ease);
  }

  .hi-tasks__button:disabled {
    cursor: default;
    opacity: 0.45;
  }

  .hi-tasks__button--ghost {
    border: 1px solid var(--line-strong);
    color: var(--fg-dim);
  }

  .hi-tasks__button--ghost:hover:not(:disabled) {
    border-color: var(--accent-line);
    background: var(--accent-wash);
    color: var(--fg);
  }

  .hi-tasks__button--danger {
    border: 1px solid var(--line-strong);
    color: var(--fg-dim);
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

  .hi-tasks__bad {
    grid-area: bad;
    padding: 0 18px 12px 32px;
    color: var(--danger);
    font-size: 12.5px;
    line-height: 1.5;
  }

  .hi-tasks__body {
    grid-area: body;
    margin: 0 18px 16px 32px;
    padding: 14px 0 0;
    border-top: 1px solid var(--line);
  }

  .hi-tasks__prose,
  .hi-tasks__none {
    color: var(--fg-dim);
    font-size: 13.5px;
    line-height: 1.65;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .hi-tasks__none,
  .hi-tasks__subject {
    color: var(--fg-dim);
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
    font-size: 11.5px;
    line-height: 1.4;
    overflow-wrap: anywhere;
  }

  .hi-tasks__empty {
    min-height: 240px;
    display: grid;
    place-content: center;
    padding: 38px;
    color: var(--fg-dim);
    text-align: center;
    font-size: 16px;
    line-height: 1.45;
    font-weight: 680;
  }

  .hi-tasks__loading {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 28px clamp(22px, 3vw, 44px);
  }

  .hi-tasks__loading span {
    width: 100%;
    height: 76px;
    border: 1px solid var(--line);
    border-radius: 8px;
    background: var(--surface);
  }

  @media (max-width: 980px) {
    .hi-tasks__row {
      grid-template-columns: minmax(0, 1fr);
      grid-template-areas:
        "toggle"
        "actions"
        "bad"
        "body";
    }

    .hi-tasks__actions {
      width: 100%;
      justify-content: flex-end;
      padding: 0 14px 12px 32px;
    }
  }

  @media (max-width: 760px) {
    .hi-tasks__header {
      min-height: 68px;
      padding: 12px 16px;
    }

    .hi-tasks__heading {
      gap: 9px;
    }

    .hi-tasks__heading h1 {
      font-size: 24px;
    }

    .hi-tasks__workspace {
      display: flex;
      flex-direction: column;
    }

    .hi-tasks__statuses {
      flex: none;
      padding: 12px 14px;
      overflow: visible;
      border-right: 0;
      border-bottom: 1px solid var(--line);
    }

    .hi-tasks__statuses-title {
      display: none;
    }

    .hi-tasks__status-list {
      display: grid;
      grid-template-columns: repeat(4, minmax(0, 1fr));
      gap: 6px;
    }

    .hi-tasks__status {
      grid-template-columns: 7px minmax(0, 1fr) auto;
      gap: 7px;
      padding: 7px 8px;
    }

    .hi-tasks__status-label {
      font-size: 12.5px;
    }

    .hi-tasks__status-count {
      min-width: 22px;
      height: 22px;
      padding: 0 5px;
    }

    .hi-tasks__content {
      flex: 1;
    }

    .hi-tasks__content-head {
      min-height: 64px;
      padding: 12px 16px;
    }

    .hi-tasks__list {
      padding: 12px 14px 0;
    }

    .hi-tasks__row-toggle {
      min-height: 72px;
      padding-right: 14px;
    }

    .hi-tasks__bad {
      padding-right: 14px;
    }

    .hi-tasks__body {
      margin-right: 14px;
    }
  }

  @media (max-width: 560px) {
    .hi-tasks__status-list {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }

    .hi-tasks__total {
      display: none;
    }

    .hi-tasks__heading {
      flex-direction: column;
      align-items: flex-start;
      gap: 2px;
    }

    .hi-tasks__heading h1 {
      font-size: 22px;
    }

    .hi-tasks__actions {
      display: grid;
      grid-template-columns: repeat(3, minmax(0, 1fr));
      padding-left: 14px;
    }

    .hi-tasks__actions:has(.hi-tasks__button:only-child) {
      display: flex;
    }

    .hi-tasks__button {
      min-width: 0;
      padding: 0 8px;
      white-space: normal;
      line-height: 1.2;
    }

    .hi-tasks__row-meta span {
      white-space: normal;
    }

    .hi-tasks__attention {
      max-width: 120px;
      text-align: right;
    }
  }
`;
