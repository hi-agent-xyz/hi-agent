// purpose: activity and usage statistics — tokens, sessions, Tools, tasks, conversation, energy, and stored inventory over time.
import { useCallback, useEffect, useState } from "react";

const T = {
  en: {
    title: "Stats",
    subtitle: "What the local record can account for",
    ranges: { "7d": "7 days", "30d": "30 days", "90d": "90 days", all: "All time" },
    refresh: "Refresh",
    loading: "Reading the record...",
    failed: "Stats could not be read.",
    retry: "Try again",
    updated: (at) => `Updated ${at}`,
    kpi: {
      tokens: "Tokens",
      sessions: "Worker sessions",
      turns: "Average turns",
      tools: "Tool calls",
      tasks: "Tasks completed",
      energy: "Energy remaining",
    },
    estimated: "Includes estimated legacy usage",
    exact: "Exact usage records",
    available: "available",
    current: "Current snapshot",
    noEnergy: "BYOK has no energy balance",
    energyReset: (at) => `resets ${at}`,
    trend: "Daily activity",
    metrics: { tokens: "Tokens", sessions: "Sessions", turns: "Turns", tools: "Tools", tasks: "Tasks" },
    chartEmpty: "No activity in this range.",
    sessions: "Sessions",
    worker: "Workers",
    resident: "Resident",
    started: "started",
    live: "live now",
    clean: "clean closes",
    lost: "restart-lost",
    avgDuration: "average duration",
    medianDuration: "median duration",
    avgTurns: "average turns",
    toolsTasks: "Tools and tasks",
    topTools: "Top Tools",
    noTools: "No Tool calls in this range.",
    commands: "commands",
    edits: "file edits",
    searches: "web searches",
    compactions: "compactions",
    taskStatus: "Task ledger now",
    created: "created",
    completed: "completed",
    cancelled: "cancelled",
    overdue: "overdue",
    stalled: "stalled doing",
    unchecked: "duties never confirmed",
    conversation: "Conversation",
    humanMessages: "Human messages",
    text: "text",
    audio: "audio",
    files: "files",
    replies: "Agent replies",
    windows: "Conversation windows",
    activeDays: "Active days",
    windowNote: "A new window starts after 30 minutes without a human message.",
    inventory: "Stored inventory",
    episodes: "Episodes",
    facets: "Facets",
    skills: "Skills",
    learned: "learned",
    people: "People",
    samples: "face / voice samples",
    drive: "Drive",
    views: "Custom views",
    coverage: "Coverage",
    tokenCoverage: (sessions, turns) => `${sessions} sessions / ${turns} turns with token data`,
    unreadable: (n) => `${n} unreadable frame${n === 1 ? "" : "s"}`,
    currentTaskHistory: "Task history is derived from each task's current lifecycle timestamps.",
    byRole: "Sessions by role",
  },
  zh: {
    title: "统计",
    subtitle: "本地记录能核对出来的使用情况",
    ranges: { "7d": "7 天", "30d": "30 天", "90d": "90 天", all: "全部" },
    refresh: "刷新",
    loading: "正在读记录...",
    failed: "统计数据读不出来。",
    retry: "再试一次",
    updated: (at) => `更新于 ${at}`,
    kpi: {
      tokens: "Tokens",
      sessions: "工作会话",
      turns: "平均轮数",
      tools: "Tool 调用",
      tasks: "完成任务",
      energy: "剩余 energy",
    },
    estimated: "包含旧记录的估算用量",
    exact: "精确用量记录",
    available: "可用",
    current: "当前快照",
    noEnergy: "BYOK 没有 energy 余额",
    energyReset: (at) => `${at} 重置`,
    trend: "每日活动",
    metrics: { tokens: "Tokens", sessions: "会话", turns: "轮数", tools: "Tools", tasks: "任务" },
    chartEmpty: "这个时间段没有活动。",
    sessions: "会话",
    worker: "工作会话",
    resident: "常驻会话",
    started: "开始",
    live: "当前在跑",
    clean: "正常结束",
    lost: "重启中断",
    avgDuration: "平均时长",
    medianDuration: "中位时长",
    avgTurns: "平均轮数",
    toolsTasks: "Tools 和任务",
    topTools: "常用 Tools",
    noTools: "这个时间段没有 Tool 调用。",
    commands: "命令",
    edits: "文件修改",
    searches: "网页搜索",
    compactions: "上下文压缩",
    taskStatus: "当前任务账本",
    created: "新建",
    completed: "完成",
    cancelled: "取消",
    overdue: "逾期",
    stalled: "doing 停滞",
    unchecked: "从未确认的值守",
    conversation: "对话",
    humanMessages: "人的消息",
    text: "文字",
    audio: "语音",
    files: "文件",
    replies: "Agent 回复",
    windows: "对话时段",
    activeDays: "活跃天数",
    windowNote: "人的消息间隔超过 30 分钟，就算新的对话时段。",
    inventory: "已存内容",
    episodes: "Episodes",
    facets: "Facets",
    skills: "Skills",
    learned: "自学",
    people: "认识的人",
    samples: "脸 / 声音样本",
    drive: "文件",
    views: "自建界面",
    coverage: "覆盖情况",
    tokenCoverage: (sessions, turns) => `${sessions} 个会话 / ${turns} 轮有 token 数据`,
    unreadable: (n) => `${n} 个帧无法读取`,
    currentTaskHistory: "任务历史来自每项任务当前保留的生命周期时间。",
    byRole: "按角色看会话",
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

const METRICS = {
  tokens: { key: "tokens", color: "var(--accent)" },
  sessions: { key: "sessions", color: "var(--accent-2)" },
  turns: { key: "turns", color: "var(--fg-dim)" },
  tools: { key: "tool_calls", color: "var(--accent)" },
  tasks: { key: "task_completions", color: "var(--accent-2)" },
};

export default function Stats() {
  const [range, setRange] = useState("30d");
  const [metric, setMetric] = useState("tokens");
  const [data, setData] = useState(null);
  const [loading, setLoading] = useState(true);
  const [failed, setFailed] = useState(false);
  const [refreshKey, setRefreshKey] = useState(0);

  const load = useCallback(() => {
    const controller = new AbortController();
    setLoading(true);
    setFailed(false);
    fetch(`/api/stats?range=${range}`, { signal: controller.signal })
      .then((response) => {
        if (!response.ok) throw new Error(`stats ${response.status}`);
        return response.json();
      })
      .then((next) => setData(next))
      .catch((error) => {
        if (error.name !== "AbortError") setFailed(true);
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [range, refreshKey]);

  useEffect(load, [load]);

  if (!data && loading) {
    return (
      <main className="hi-stats">
        <style>{CSS}</style>
        <Header range={range} setRange={setRange} loading />
        <div className="hi-stats__state">{L.loading}</div>
      </main>
    );
  }

  if (!data && failed) {
    return (
      <main className="hi-stats">
        <style>{CSS}</style>
        <Header range={range} setRange={setRange} />
        <div className="hi-stats__state" role="alert">
          <strong>{L.failed}</strong>
          <button type="button" className="hi-stats__button" onClick={() => setRefreshKey((key) => key + 1)}>
            {L.retry}
          </button>
        </div>
      </main>
    );
  }

  const stats = data || {};
  const summary = stats.summary || {};
  const sessions = summary.sessions || {};
  const worker = sessions.worker || {};
  const tools = summary.tools || {};
  const tasks = summary.tasks || {};
  const conversation = summary.conversation || {};
  const energy = summary.energy || {};
  const coverage = stats.coverage || {};
  const inventory = stats.inventory || {};
  const breakdowns = stats.breakdowns || {};
  const energyValue = energy.available ? `${fmt(energy.remaining)} / ${fmt(energy.total)}` : "—";
  const energyMeta = energy.available
    ? [energy.tier, energy.resets_at ? L.energyReset(shortDateTime(energy.resets_at)) : null].filter(Boolean).join(" · ")
    : L.noEnergy;

  const kpis = [
    { label: L.kpi.tokens, value: compact(summary.tokens?.total), meta: summary.tokens?.estimated ? L.estimated : L.exact },
    { label: L.kpi.sessions, value: fmt(worker.started), meta: `${fmt(worker.clean_closed)} ${L.clean}` },
    { label: L.kpi.turns, value: decimal(worker.average_turns), meta: `${fmt(worker.turn_samples)} ${L.sessions.toLowerCase()}` },
    { label: L.kpi.tools, value: fmt(tools.calls), meta: `${fmt(tools.available_distinct)} ${L.available}` },
    { label: L.kpi.tasks, value: fmt(tasks.completed), meta: `${fmt(tasks.created)} ${L.created}` },
    { label: L.kpi.energy, value: energyValue, meta: energyMeta, current: true },
  ];

  return (
    <main className="hi-stats">
      <style>{CSS}</style>
      <Header
        range={range}
        setRange={setRange}
        loading={loading}
        updated={stats.period?.generated_at}
        onRefresh={() => setRefreshKey((key) => key + 1)}
      />

      {failed && <div className="hi-stats__inline-error" role="alert">{L.failed}</div>}

      <section className="hi-stats__kpis" aria-label={L.title}>
        {kpis.map((item) => (
          <div className="hi-stats__kpi" key={item.label}>
            <div className="hi-stats__kpi-label">
              {item.label}
              {item.current && <span className="hi-stats__snapshot">{L.current}</span>}
            </div>
            <div className="hi-stats__kpi-value">{item.value}</div>
            <div className="hi-stats__kpi-meta">{item.meta || "\u00a0"}</div>
          </div>
        ))}
      </section>

      <section className="hi-stats__band">
        <div className="hi-stats__section-head">
          <h2>{L.trend}</h2>
          <div className="hi-stats__segments hi-stats__segments--small" role="group" aria-label={L.trend}>
            {Object.keys(METRICS).map((name) => (
              <button key={name} type="button" aria-pressed={metric === name}
                onClick={() => setMetric(name)}>
                {L.metrics[name]}
              </button>
            ))}
          </div>
        </div>
        <ActivityChart series={stats.series || []} metric={metric} />
      </section>

      <section className="hi-stats__split">
        <div className="hi-stats__section">
          <div className="hi-stats__section-head">
            <h2>{L.sessions}</h2>
            <span>{fmt(sessions.live)} {L.live}</span>
          </div>
          <div className="hi-stats__session-grid">
            <SessionColumn title={L.worker} group={worker} />
            <SessionColumn title={L.resident} group={sessions.resident || {}} />
          </div>
          <div className="hi-stats__quality">
            <StatLine label={L.clean} value={fmt(sessions.clean_closed)} />
            <StatLine label={L.lost} value={fmt(sessions.restart_lost)} danger={sessions.restart_lost > 0} />
          </div>
          <RoleBars rows={breakdowns.sessions_by_role || []} />
        </div>

        <div className="hi-stats__section">
          <div className="hi-stats__section-head"><h2>{L.toolsTasks}</h2></div>
          <div className="hi-stats__tools">
            <div>
              <h3>{L.topTools}</h3>
              <RankedBars rows={(breakdowns.tools_by_name || []).slice(0, 7)} />
            </div>
            <div className="hi-stats__acts">
              <TinyStat label={L.commands} value={tools.commands} />
              <TinyStat label={L.edits} value={tools.edits} />
              <TinyStat label={L.searches} value={tools.web_searches} />
              <TinyStat label={L.compactions} value={tools.context_compactions} />
            </div>
          </div>
          <div className="hi-stats__task-block">
            <h3>{L.taskStatus}</h3>
            <StatusBar rows={breakdowns.tasks_by_status || []} />
            <div className="hi-stats__acts hi-stats__acts--tasks">
              <TinyStat label={L.overdue} value={tasks.overdue} alert={tasks.overdue > 0} />
              <TinyStat label={L.stalled} value={tasks.stalled_doing} alert={tasks.stalled_doing > 0} />
              <TinyStat label={L.unchecked} value={tasks.serving_never_confirmed} alert={tasks.serving_never_confirmed > 0} />
              <TinyStat label={L.cancelled} value={tasks.cancelled} />
            </div>
          </div>
        </div>
      </section>

      <section className="hi-stats__band hi-stats__conversation">
        <div className="hi-stats__section-head"><h2>{L.conversation}</h2></div>
        <div className="hi-stats__conversation-grid">
          <BigStat label={L.humanMessages} value={conversation.human_messages}
            note={`${fmt(conversation.human_text_messages)} ${L.text} · ${fmt(conversation.human_audio_messages)} ${L.audio} · ${fmt(conversation.handed_files)} ${L.files}`} />
          <BigStat label={L.replies} value={conversation.agent_text_replies} />
          <BigStat label={L.windows} value={conversation.conversation_windows} note={L.windowNote} />
          <BigStat label={L.activeDays} value={conversation.active_days} />
        </div>
      </section>

      <section className="hi-stats__inventory">
        <div className="hi-stats__section-head"><h2>{L.inventory}</h2></div>
        <div className="hi-stats__inventory-grid">
          <InventoryStat label={L.episodes} value={inventory.episodes} />
          <InventoryStat label={L.facets} value={inventory.facets} />
          <InventoryStat label={L.skills} value={inventory.skills?.total}
            note={`${fmt(inventory.skills?.learned)} ${L.learned}`} />
          <InventoryStat label={L.people} value={inventory.people?.clusters}
            note={`${fmt(inventory.people?.face_samples)} / ${fmt(inventory.people?.voice_samples)} ${L.samples}`} />
          <InventoryStat label={L.drive} value={inventory.drive?.files}
            note={bytes(inventory.drive?.bytes)} />
          <InventoryStat label={L.views} value={inventory.custom_views} />
        </div>
      </section>

      <footer className="hi-stats__coverage">
        <strong>{L.coverage}</strong>
        <span>{L.tokenCoverage(coverage.token_sessions || 0, coverage.token_turns || 0)}</span>
        {coverage.unreadable_frames > 0 && <span className="hi-stats__danger">{L.unreadable(coverage.unreadable_frames)}</span>}
        <span>{L.currentTaskHistory}</span>
      </footer>
    </main>
  );
}

function Header({ range, setRange, loading, updated, onRefresh }) {
  return (
    <header className="hi-stats__header">
      <div>
        <h1>{L.title}</h1>
        <p>{L.subtitle}</p>
      </div>
      <div className="hi-stats__header-actions">
        <div className="hi-stats__segments" role="group" aria-label={L.title}>
          {["7d", "30d", "90d", "all"].map((value) => (
            <button key={value} type="button" aria-pressed={range === value}
              onClick={() => setRange(value)}>
              {L.ranges[value]}
            </button>
          ))}
        </div>
        <button type="button" className="hi-stats__button" onClick={onRefresh} disabled={!onRefresh || loading}>
          <span aria-hidden>{loading ? "..." : "↻"}</span>
          {L.refresh}
        </button>
        {updated && <span className="hi-stats__updated">{L.updated(shortDateTime(updated))}</span>}
      </div>
    </header>
  );
}

function ActivityChart({ series, metric }) {
  const spec = METRICS[metric];
  const values = series.map((row) => Number(row[spec.key]) || 0);
  const max = Math.max(0, ...values);
  const width = 900;
  const height = 220;
  const left = 54;
  const right = 16;
  const top = 18;
  const bottom = 34;
  const chartWidth = width - left - right;
  const chartHeight = height - top - bottom;
  const coords = values.map((value, index) => {
    const x = values.length <= 1 ? left + chartWidth / 2 : left + (index / (values.length - 1)) * chartWidth;
    const y = top + chartHeight - (max ? value / max : 0) * chartHeight;
    return [x, y];
  });
  const path = coords.map(([x, y], index) => `${index ? "L" : "M"}${x.toFixed(1)},${y.toFixed(1)}`).join(" ");
  const ticks = [0, 0.5, 1].map((ratio) => ({
    ratio,
    value: Math.round(max * (1 - ratio)),
    y: top + chartHeight * ratio,
  }));
  const dateIndexes = series.length <= 1 ? [0] : [...new Set([0, Math.floor((series.length - 1) / 2), series.length - 1])];

  return (
    <div className="hi-stats__chart">
      {max === 0 && <div className="hi-stats__chart-empty">{L.chartEmpty}</div>}
      <svg viewBox={`0 0 ${width} ${height}`} role="img" aria-label={`${L.trend}: ${L.metrics[metric]}`}>
        {ticks.map((tick) => (
          <g key={tick.ratio}>
            <line x1={left} x2={width - right} y1={tick.y} y2={tick.y} className="hi-stats__gridline" />
            <text x={left - 10} y={tick.y + 4} textAnchor="end" className="hi-stats__axis">
              {compact(tick.value)}
            </text>
          </g>
        ))}
        {max > 0 && <path d={path} fill="none" stroke={spec.color} strokeWidth="3" strokeLinecap="round" strokeLinejoin="round" />}
        {coords.map(([x, y], index) => (
          <circle key={series[index]?.date || index} cx={x} cy={y} r={series.length <= 31 ? 3.5 : 1.5}
            fill={spec.color}>
            <title>{`${series[index]?.date}: ${fmt(values[index])}`}</title>
          </circle>
        ))}
        {dateIndexes.map((index) => (
          <text key={index} x={coords[index]?.[0] || left} y={height - 8}
            textAnchor={index === 0 ? "start" : index === series.length - 1 ? "end" : "middle"}
            className="hi-stats__axis">
            {shortDate(series[index]?.date)}
          </text>
        ))}
      </svg>
      <table className="hi-stats__sr-only">
        <caption>{`${L.trend}: ${L.metrics[metric]}`}</caption>
        <thead><tr><th>Date</th><th>{L.metrics[metric]}</th></tr></thead>
        <tbody>{series.map((row) => <tr key={row.date}><td>{row.date}</td><td>{fmt(row[spec.key])}</td></tr>)}</tbody>
      </table>
    </div>
  );
}

function SessionColumn({ title, group }) {
  return (
    <div className="hi-stats__session-column">
      <h3>{title}</h3>
      <div className="hi-stats__session-main">{fmt(group.started)}</div>
      <div className="hi-stats__muted">{L.started} · {fmt(group.live)} {L.live}</div>
      <dl>
        <div><dt>{L.avgDuration}</dt><dd>{duration(group.average_duration_seconds)}</dd></div>
        <div><dt>{L.medianDuration}</dt><dd>{duration(group.median_duration_seconds)}</dd></div>
        <div><dt>{L.avgTurns}</dt><dd>{decimal(group.average_turns)}</dd></div>
      </dl>
    </div>
  );
}

function RoleBars({ rows }) {
  if (!rows.length) return null;
  const max = Math.max(...rows.map((row) => row.count), 1);
  return (
    <div className="hi-stats__roles" aria-label={L.byRole}>
      {rows.map((row) => (
        <div className="hi-stats__role" key={row.name}>
          <span>{roleName(row.name)}</span>
          <i style={{ width: `${Math.max(4, (row.count / max) * 100)}%` }} />
          <b>{fmt(row.count)}</b>
        </div>
      ))}
    </div>
  );
}

function RankedBars({ rows }) {
  if (!rows.length) return <div className="hi-stats__muted">{L.noTools}</div>;
  const max = Math.max(...rows.map((row) => row.count), 1);
  return (
    <div className="hi-stats__ranked">
      {rows.map((row) => (
        <div className="hi-stats__rank" key={row.name}>
          <span title={row.name}>{row.name.replace(/^hi_/, "")}</span>
          <div><i style={{ width: `${Math.max(3, (row.count / max) * 100)}%` }} /></div>
          <b>{fmt(row.count)}</b>
        </div>
      ))}
    </div>
  );
}

function StatusBar({ rows }) {
  const total = rows.reduce((sum, row) => sum + row.count, 0);
  if (!total) return <div className="hi-stats__muted">0</div>;
  return (
    <>
      <div className="hi-stats__statusbar" role="img" aria-label={L.taskStatus}>
        {rows.map((row) => (
          <i key={row.name} data-status={row.name} style={{ width: `${(row.count / total) * 100}%` }}
            title={`${row.name}: ${row.count}`} />
        ))}
      </div>
      <div className="hi-stats__status-legend">
        {rows.map((row) => <span key={row.name}><i data-status={row.name} />{row.name} {fmt(row.count)}</span>)}
      </div>
    </>
  );
}

function StatLine({ label, value, danger }) {
  return <div><span>{label}</span><b className={danger ? "hi-stats__danger" : ""}>{value}</b></div>;
}

function TinyStat({ label, value, alert }) {
  return <div className={alert ? "hi-stats__tiny hi-stats__tiny--alert" : "hi-stats__tiny"}>
    <b>{fmt(value)}</b><span>{label}</span>
  </div>;
}

function BigStat({ label, value, note }) {
  return <div className="hi-stats__big-stat"><span>{label}</span><b>{fmt(value)}</b>{note && <small>{note}</small>}</div>;
}

function InventoryStat({ label, value, note }) {
  return <div className="hi-stats__inventory-stat"><b>{fmt(value)}</b><span>{label}</span>{note && <small>{note}</small>}</div>;
}

function fmt(value) {
  const number = Number(value);
  return Number.isFinite(number) ? number.toLocaleString() : "0";
}

function compact(value) {
  const number = Number(value) || 0;
  if (number < 1000) return fmt(number);
  if (number < 1000000) return `${(number / 1000).toFixed(number >= 10000 ? 0 : 1)}K`;
  if (number < 1000000000) return `${(number / 1000000).toFixed(number >= 10000000 ? 0 : 1)}M`;
  return `${(number / 1000000000).toFixed(1)}B`;
}

function decimal(value) {
  const number = Number(value);
  return Number.isFinite(number) ? number.toFixed(1) : "—";
}

function duration(seconds) {
  const value = Number(seconds);
  if (!Number.isFinite(value)) return "—";
  if (value < 60) return `${Math.round(value)}s`;
  if (value < 3600) return `${Math.round(value / 60)}m`;
  if (value < 86400) return `${(value / 3600).toFixed(value < 36000 ? 1 : 0)}h`;
  return `${(value / 86400).toFixed(1)}d`;
}

function bytes(value) {
  const number = Number(value) || 0;
  if (number < 1024) return `${number} B`;
  if (number < 1048576) return `${Math.round(number / 1024)} KB`;
  if (number < 1073741824) return `${(number / 1048576).toFixed(1)} MB`;
  return `${(number / 1073741824).toFixed(1)} GB`;
}

function shortDate(value) {
  if (!value) return "";
  const date = new Date(`${value}T00:00:00Z`);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

function shortDateTime(value) {
  const date = new Date(value);
  return Number.isNaN(date.getTime())
    ? value
    : date.toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
}

function roleName(value) {
  return value ? value.charAt(0).toUpperCase() + value.slice(1) : "Unknown";
}

const CSS = `
  .hi-stats {
    width: 100%;
    height: 100%;
    min-height: 0;
    overflow-y: auto;
    box-sizing: border-box;
    padding: max(28px, var(--hi-safe-top)) clamp(18px, 3vw, 44px) 128px;
    color: var(--fg);
    font-family: var(--font-display);
    container-type: inline-size;
  }
  .hi-stats * { box-sizing: border-box; letter-spacing: 0; }
  .hi-stats button { font: inherit; }
  .hi-stats__header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 24px;
    margin-bottom: 24px;
  }
  .hi-stats__header h1 {
    margin: 0;
    font-size: 30px;
    line-height: 1.1;
    font-weight: 800;
  }
  .hi-stats__header p {
    margin: 7px 0 0;
    color: var(--fg-mute);
    font-size: 13px;
  }
  .hi-stats__header-actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 10px;
    flex-wrap: wrap;
  }
  .hi-stats__segments {
    display: inline-flex;
    min-height: 36px;
    padding: 3px;
    border: 1px solid var(--surface-border);
    border-radius: 7px;
    background: var(--surface);
  }
  .hi-stats__segments button {
    min-width: 62px;
    padding: 6px 10px;
    border: 0;
    border-radius: 5px;
    color: var(--fg-mute);
    background: transparent;
    cursor: pointer;
    font-size: 12px;
    font-weight: 700;
  }
  .hi-stats__segments button[aria-pressed="true"] {
    color: var(--fg);
    background: var(--surface-strong);
    box-shadow: 0 1px 2px var(--shadow);
  }
  .hi-stats__segments--small { min-height: 32px; }
  .hi-stats__segments--small button { min-width: 54px; padding: 4px 8px; }
  .hi-stats__button {
    min-height: 36px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 7px;
    padding: 0 12px;
    border: 1px solid var(--surface-border);
    border-radius: 7px;
    color: var(--fg-dim);
    background: var(--surface-strong);
    cursor: pointer;
    font-size: 12px;
    font-weight: 700;
  }
  .hi-stats__button:disabled { cursor: default; opacity: .55; }
  .hi-stats__updated {
    width: 100%;
    color: var(--fg-mute);
    text-align: right;
    font-size: 11px;
  }
  .hi-stats__state {
    min-height: 260px;
    display: grid;
    place-items: center;
    gap: 16px;
    color: var(--fg-mute);
    text-align: center;
    font-size: 14px;
  }
  .hi-stats__state strong { color: var(--fg-dim); }
  .hi-stats__inline-error {
    margin: -10px 0 14px;
    color: var(--danger);
    font-size: 12px;
  }
  .hi-stats__kpis {
    display: grid;
    grid-template-columns: repeat(6, minmax(0, 1fr));
    border-top: 1px solid var(--line);
    border-bottom: 1px solid var(--line);
  }
  .hi-stats__kpi {
    min-width: 0;
    padding: 18px 16px 17px;
    border-right: 1px solid var(--line);
  }
  .hi-stats__kpi:first-child { padding-left: 0; }
  .hi-stats__kpi:last-child { padding-right: 0; border-right: 0; }
  .hi-stats__kpi-label {
    min-height: 18px;
    display: flex;
    align-items: center;
    gap: 7px;
    color: var(--fg-mute);
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
  }
  .hi-stats__snapshot {
    padding: 2px 5px;
    border: 1px solid var(--accent-line);
    border-radius: 4px;
    color: var(--accent);
    text-transform: none;
    font-size: 9px;
  }
  .hi-stats__kpi-value {
    margin-top: 8px;
    overflow: hidden;
    color: var(--fg);
    font-family: var(--font-mono);
    font-size: 25px;
    font-weight: 750;
    line-height: 1.1;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hi-stats__kpi-meta {
    min-height: 16px;
    margin-top: 7px;
    overflow: hidden;
    color: var(--fg-mute);
    font-size: 10.5px;
    line-height: 1.4;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hi-stats__band {
    padding: 26px 0;
    border-bottom: 1px solid var(--line);
  }
  .hi-stats__section-head {
    min-height: 34px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
  }
  .hi-stats__section-head h2 {
    margin: 0;
    font-size: 16px;
    line-height: 1.25;
    font-weight: 800;
  }
  .hi-stats__section-head > span {
    color: var(--fg-mute);
    font-size: 12px;
  }
  .hi-stats__chart {
    position: relative;
    width: 100%;
    min-height: 220px;
    margin-top: 12px;
  }
  .hi-stats__chart svg { display: block; width: 100%; height: 220px; overflow: visible; }
  .hi-stats__gridline { stroke: var(--line); stroke-width: 1; }
  .hi-stats__axis { fill: var(--fg-mute); font-family: var(--font-mono); font-size: 10px; }
  .hi-stats__chart-empty {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    color: var(--fg-mute);
    font-size: 13px;
  }
  .hi-stats__split {
    display: grid;
    grid-template-columns: minmax(0, .92fr) minmax(0, 1.08fr);
    border-bottom: 1px solid var(--line);
  }
  .hi-stats__section {
    min-width: 0;
    padding: 26px 28px 28px 0;
  }
  .hi-stats__section + .hi-stats__section {
    padding-right: 0;
    padding-left: 28px;
    border-left: 1px solid var(--line);
  }
  .hi-stats__section h3 {
    margin: 0 0 12px;
    color: var(--fg-dim);
    font-size: 12px;
    font-weight: 800;
  }
  .hi-stats__session-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 1px;
    margin-top: 14px;
    background: var(--line);
  }
  .hi-stats__session-column {
    min-width: 0;
    padding: 16px;
    background: var(--surface);
  }
  .hi-stats__session-column h3 { margin-bottom: 8px; }
  .hi-stats__session-main {
    font-family: var(--font-mono);
    font-size: 27px;
    font-weight: 750;
  }
  .hi-stats__muted { color: var(--fg-mute); font-size: 11px; line-height: 1.45; }
  .hi-stats__session-column dl { margin: 16px 0 0; }
  .hi-stats__session-column dl div {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    padding: 6px 0;
    border-top: 1px solid var(--line);
    font-size: 11px;
  }
  .hi-stats__session-column dt { color: var(--fg-mute); }
  .hi-stats__session-column dd { margin: 0; font-family: var(--font-mono); font-weight: 700; }
  .hi-stats__quality {
    display: flex;
    gap: 24px;
    padding: 13px 2px;
    border-bottom: 1px solid var(--line);
  }
  .hi-stats__quality div { display: flex; align-items: baseline; gap: 8px; font-size: 11px; }
  .hi-stats__quality span { color: var(--fg-mute); }
  .hi-stats__roles { margin-top: 13px; display: grid; gap: 7px; }
  .hi-stats__role {
    display: grid;
    grid-template-columns: 78px minmax(0, 1fr) 30px;
    align-items: center;
    gap: 9px;
    min-height: 17px;
    font-size: 10.5px;
  }
  .hi-stats__role span { overflow: hidden; color: var(--fg-mute); text-overflow: ellipsis; white-space: nowrap; }
  .hi-stats__role i { display: block; height: 5px; border-radius: 2px; background: var(--accent-2); }
  .hi-stats__role b { text-align: right; font-family: var(--font-mono); }
  .hi-stats__tools {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 160px;
    gap: 24px;
    margin-top: 14px;
  }
  .hi-stats__ranked { display: grid; gap: 8px; }
  .hi-stats__rank {
    display: grid;
    grid-template-columns: minmax(80px, 1.1fr) minmax(90px, 1fr) 32px;
    align-items: center;
    gap: 9px;
    min-height: 17px;
    font-size: 10.5px;
  }
  .hi-stats__rank > span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .hi-stats__rank > div { height: 6px; overflow: hidden; border-radius: 2px; background: var(--surface); }
  .hi-stats__rank i { display: block; height: 100%; border-radius: 2px; background: var(--accent); }
  .hi-stats__rank b { text-align: right; font-family: var(--font-mono); }
  .hi-stats__acts {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    align-content: start;
    gap: 1px;
    background: var(--line);
  }
  .hi-stats__tiny {
    min-width: 0;
    min-height: 58px;
    padding: 10px;
    background: var(--surface);
  }
  .hi-stats__tiny b { display: block; font-family: var(--font-mono); font-size: 16px; }
  .hi-stats__tiny span { display: block; margin-top: 4px; color: var(--fg-mute); font-size: 9.5px; line-height: 1.25; }
  .hi-stats__tiny--alert b { color: var(--danger); }
  .hi-stats__task-block {
    margin-top: 24px;
    padding-top: 20px;
    border-top: 1px solid var(--line);
  }
  .hi-stats__statusbar {
    width: 100%;
    height: 9px;
    display: flex;
    overflow: hidden;
    border-radius: 3px;
    background: var(--surface);
  }
  .hi-stats__statusbar i { display: block; height: 100%; min-width: 2px; }
  [data-status="todo"] { background: var(--fg-mute); }
  [data-status="doing"] { background: var(--accent); }
  [data-status="serving"] { background: var(--accent-2); }
  [data-status="done"] { background: var(--fg-dim); }
  [data-status="cancelled"] { background: var(--line-strong); }
  .hi-stats__status-legend {
    display: flex;
    flex-wrap: wrap;
    gap: 7px 14px;
    margin-top: 9px;
    color: var(--fg-mute);
    font-size: 9.5px;
  }
  .hi-stats__status-legend span { display: inline-flex; align-items: center; gap: 5px; }
  .hi-stats__status-legend i { width: 6px; height: 6px; border-radius: 2px; }
  .hi-stats__acts--tasks { grid-template-columns: repeat(4, minmax(0, 1fr)); margin-top: 14px; }
  .hi-stats__conversation { padding-bottom: 28px; }
  .hi-stats__conversation-grid {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 24px;
    margin-top: 14px;
  }
  .hi-stats__big-stat { min-width: 0; }
  .hi-stats__big-stat > span { color: var(--fg-mute); font-size: 11px; font-weight: 700; }
  .hi-stats__big-stat > b {
    display: block;
    margin-top: 7px;
    font-family: var(--font-mono);
    font-size: 24px;
  }
  .hi-stats__big-stat small {
    display: block;
    margin-top: 6px;
    color: var(--fg-mute);
    font-size: 9.5px;
    line-height: 1.4;
  }
  .hi-stats__inventory { padding: 25px 0 21px; }
  .hi-stats__inventory-grid {
    display: grid;
    grid-template-columns: repeat(6, minmax(0, 1fr));
    gap: 1px;
    margin-top: 12px;
    background: var(--line);
  }
  .hi-stats__inventory-stat {
    min-width: 0;
    min-height: 84px;
    padding: 14px;
    background: var(--surface);
  }
  .hi-stats__inventory-stat b { display: block; font-family: var(--font-mono); font-size: 19px; }
  .hi-stats__inventory-stat span { display: block; margin-top: 6px; color: var(--fg-dim); font-size: 10.5px; font-weight: 700; }
  .hi-stats__inventory-stat small { display: block; margin-top: 4px; color: var(--fg-mute); font-size: 9px; line-height: 1.3; }
  .hi-stats__coverage {
    display: flex;
    align-items: baseline;
    gap: 8px 18px;
    flex-wrap: wrap;
    padding-top: 14px;
    color: var(--fg-mute);
    font-size: 9.5px;
    line-height: 1.4;
  }
  .hi-stats__coverage strong { color: var(--fg-dim); font-size: 10px; }
  .hi-stats__danger { color: var(--danger); }
  .hi-stats__sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }
  @container (max-width: 900px) {
    .hi-stats__kpis { grid-template-columns: repeat(3, minmax(0, 1fr)); }
    .hi-stats__kpi:nth-child(3) { border-right: 0; }
    .hi-stats__kpi:nth-child(-n+3) { border-bottom: 1px solid var(--line); }
    .hi-stats__kpi:nth-child(4) { padding-left: 0; }
    .hi-stats__split { grid-template-columns: 1fr; }
    .hi-stats__section { padding-right: 0; }
    .hi-stats__section + .hi-stats__section { padding-left: 0; border-top: 1px solid var(--line); border-left: 0; }
    .hi-stats__conversation-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
    .hi-stats__inventory-grid { grid-template-columns: repeat(3, minmax(0, 1fr)); }
  }
  @container (max-width: 620px) {
    .hi-stats { padding-top: 22px; }
    .hi-stats__header { flex-direction: column; align-items: stretch; gap: 16px; }
    .hi-stats__header-actions { justify-content: flex-start; }
    .hi-stats__segments { width: 100%; }
    .hi-stats__segments button { min-width: 0; flex: 1; }
    .hi-stats__updated { text-align: left; }
    .hi-stats__kpis { grid-template-columns: repeat(2, minmax(0, 1fr)); }
    .hi-stats__kpi { padding: 15px 12px; border-right: 1px solid var(--line); border-bottom: 1px solid var(--line); }
    .hi-stats__kpi:nth-child(odd) { padding-left: 0; }
    .hi-stats__kpi:nth-child(even) { padding-right: 0; border-right: 0; }
    .hi-stats__kpi:nth-last-child(-n+2) { border-bottom: 0; }
    .hi-stats__section-head { align-items: flex-start; flex-direction: column; }
    .hi-stats__segments--small { width: 100%; }
    .hi-stats__session-grid { grid-template-columns: 1fr; }
    .hi-stats__tools { grid-template-columns: 1fr; }
    .hi-stats__acts--tasks { grid-template-columns: repeat(2, minmax(0, 1fr)); }
    .hi-stats__conversation-grid { grid-template-columns: 1fr 1fr; gap: 22px 16px; }
    .hi-stats__inventory-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
    .hi-stats__chart svg { height: 190px; }
  }
`;
