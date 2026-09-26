// purpose: live host system status — CPU, memory, per-interface network traffic, available temperature sensors and batteries.
import { useState } from "react";
import { useLive, TEMPO } from "@hi/core";

const T = {
  en: {
    title: "System status", subtitle: "The machine running Hi Agent · whole-system readings",
    loading: "Reading system status…", failed: "Unable to refresh. Any values below are the last successful reading, not live data.",
    refresh: "Refresh", updated: "Sampled", cadence: "Refreshes every 2 seconds while visible",
    cpu: "CPU", cores: "logical CPUs", memory: "Memory", swap: "Swap used",
    network: "Network traffic", down: "Download", up: "Upload", unavailable: "Unavailable on this device",
    temperature: "Temperature sensors", battery: "Battery",
    power: "Battery energy flow", states: { charging: "Charging", discharging: "Discharging", full: "Full", empty: "Empty", unknown: "Unknown" },
    sensorNote: "Sensor availability depends on hardware and OS permissions. A missing reading does not mean zero.",
    networkNote: "Measured traffic, not bandwidth capacity. Interfaces are shown separately because VPNs and virtual interfaces can count the same traffic twice.",
    powerNote: "Battery energy flow is not whole-machine wall power or Hi Agent’s power use. Desktops may have no battery reading.",
    scope: "Includes other apps. In containers, OS-visible readings may differ from container resource limits.",
    high: "High utilization", busy: "Elevated utilization", normal: "Current utilization",
  },
  zh: {
    title: "系统状态", subtitle: "运行 Hi Agent 的机器 · 系统整体读数",
    loading: "正在读取系统状态…", failed: "刷新失败。下方如有数据，是上次成功读取的结果，并非实时状态。",
    refresh: "刷新", updated: "采样于", cadence: "页面可见时每 2 秒刷新",
    cpu: "CPU", cores: "个逻辑核心", memory: "内存", swap: "已用交换空间",
    network: "网络流量", down: "下载", up: "上传", unavailable: "此设备暂无法提供读数",
    temperature: "温度传感器", battery: "电池",
    power: "电池充放电功率", states: { charging: "充电中", discharging: "放电中", full: "已充满", empty: "已耗尽", unknown: "状态未知" },
    sensorNote: "可读取的传感器取决于硬件和系统权限。没有读数不代表数值为零。",
    networkNote: "这里是实际流量，不是可用带宽。VPN 和虚拟网卡可能重复统计同一流量，因此按网卡分别显示。",
    powerNote: "电池充放电功率不等于整机插座功耗，也不是 Hi Agent 单独的耗电量。台式机可能没有电池读数。",
    scope: "包含其他应用的资源使用。容器中的系统读数可能与容器资源限额不同。",
    high: "使用率较高", busy: "使用率有所升高", normal: "当前使用率",
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
const known = (v) => typeof v === "number" && Number.isFinite(v);
function bytes(value) {
  if (!known(value)) return "—";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let n = value, unit = 0;
  while (n >= 1024 && unit < units.length - 1) { n /= 1024; unit++; }
  return `${n.toLocaleString(undefined, { maximumFractionDigits: unit ? 1 : 0 })} ${units[unit]}`;
}
function percent(value) { return known(value) ? `${value.toFixed(1)}%` : "—"; }

export default function SystemStatus() {
  const [data, setData] = useState(null);
  const [failed, setFailed] = useState(false);
  const [loading, setLoading] = useState(true);
  const [refreshKey, setRefreshKey] = useState(0);
  useLive(async () => {
    setLoading(true);
    try {
      const response = await fetch("/api/system/status", { cache: "no-store", signal: AbortSignal.timeout(10000) });
      if (!response.ok) throw new Error(`system status ${response.status}`);
      setData(await response.json());
      setFailed(false);
    } catch { setFailed(true); }
    finally { setLoading(false); }
  }, { period: TEMPO.watching, subject: refreshKey });
  const memoryPercent = known(data?.memory_used_bytes) && data?.memory_total_bytes > 0
    ? data.memory_used_bytes / data.memory_total_bytes * 100 : null;
  return <main className="hi-system">
    <style>{CSS}</style>
    <header>
      <div><h1>{L.title}</h1><p>{L.subtitle}{data ? ` · ${data.platform}` : ""}</p></div>
      <button type="button" disabled={loading} onClick={() => setRefreshKey(k => k + 1)}>{L.refresh}</button>
    </header>
    <p className="hi-system-meta">{data ? `${L.updated} ${new Date(data.sampled_at).toLocaleTimeString()} · ` : ""}{L.cadence}</p>
    {failed && <p role="alert" className="hi-system-error">{L.failed}</p>}
    {!data && !failed && <p role="status">{L.loading}</p>}
    {data && <>
      <div className="hi-system-grid">
        <Metric label={L.cpu} value={data.cpu_percent} note={`${data.logical_cpus} ${L.cores}`} />
        <Metric label={L.memory} value={memoryPercent} note={`${bytes(data.memory_used_bytes)} / ${bytes(data.memory_total_bytes)}`} />
      </div>
      <p className="hi-system-meta">{L.swap}: {bytes(data.swap_used_bytes)} · {L.scope}</p>
      <section><h2>{L.network}</h2>
        {data.networks.length ? data.networks.map(n => <div className="hi-system-row" key={n.name}>
          <strong>{n.name}</strong><div><span>{L.down} <b>{bytes(n.received_bytes_per_second)}/s</b></span><span>{L.up} <b>{bytes(n.transmitted_bytes_per_second)}/s</b></span></div>
        </div>) : <p>{L.unavailable}</p>}
        <p className="hi-system-meta">{L.networkNote}</p>
      </section>
      <div className="hi-system-grid">
        <section><h2>{L.temperature}</h2>
          {data.temperatures.length ? data.temperatures.map((t, i) => <div className="hi-system-row" key={i}><span>{t.label}</span><b>{t.celsius.toFixed(1)} °C</b></div>) : <p>{L.unavailable}</p>}
          <p className="hi-system-meta">{L.sensorNote}</p>
        </section>
        <section><h2>{L.battery}</h2>
          {data.batteries.length ? data.batteries.map((b, i) => <div className="hi-system-battery" key={i}>
            <div className="hi-system-row"><strong>{L.battery} {i + 1}</strong><b>{percent(b.percent)} · {L.states[b.state] || L.states.unknown}</b></div>
            <div className="hi-system-row"><span>{L.power}</span><b>{known(b.energy_rate_watts) ? `${b.energy_rate_watts.toFixed(1)} W` : "—"}</b></div>
          </div>) : <p>{L.unavailable}</p>}
          <p className="hi-system-meta">{L.powerNote}</p>
        </section>
      </div>
    </>}
  </main>;
}
function Metric({ label, value, note }) {
  return <section className="hi-system-metric">
    <h2>{label}</h2><strong>{percent(value)}</strong>
    {known(value) && <meter min="0" max="100" value={Math.min(100, Math.max(0, value))} aria-label={label} />}
    <p>{known(value) ? value >= 90 ? L.high : value >= 70 ? L.busy : L.normal : L.unavailable}</p>
    <p className="hi-system-meta">{note}</p>
  </section>;
}
const CSS = `
.hi-system{box-sizing:border-box;width:100%;max-width:1080px;margin:auto;padding:calc(env(safe-area-inset-top,0px) + 32px) 24px 128px;color:var(--fg);font-family:var(--font-sans,system-ui);container-type:inline-size}
.hi-system *{box-sizing:border-box}.hi-system header{display:flex;gap:16px;align-items:center;justify-content:space-between;flex-wrap:wrap}
.hi-system h1{font-size:28px;margin:0 0 8px}.hi-system h2{font-size:16px;margin:0 0 16px}.hi-system p{line-height:1.6;margin:8px 0}
.hi-system button{min-height:44px;padding:8px 18px;border:1px solid var(--line);border-radius:12px;background:var(--bg-1);color:var(--fg);font:inherit;cursor:pointer}
.hi-system button:disabled{opacity:.5;cursor:default}.hi-system button:focus-visible{outline:2px solid var(--accent);outline-offset:3px}
.hi-system-grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:16px;margin-top:20px}
.hi-system section{min-width:0;border:1px solid var(--line);border-radius:16px;padding:20px;margin-top:20px;background:var(--bg-1)}
.hi-system-grid section{margin-top:0}.hi-system-meta{font-size:13px;color:var(--fg-dim);overflow-wrap:anywhere}
.hi-system-metric>strong{font-size:36px;font-variant-numeric:tabular-nums}.hi-system-metric meter{display:block;width:100%;height:12px;margin:16px 0;accent-color:var(--accent)}
.hi-system-row{display:flex;justify-content:space-between;align-items:baseline;flex-wrap:wrap;gap:8px 16px;padding:12px 0;border-bottom:1px solid var(--line);overflow-wrap:anywhere}
.hi-system-row>div{display:flex;flex-wrap:wrap;gap:8px 24px}.hi-system-row span{font-size:14px}.hi-system b{font-variant-numeric:tabular-nums;font-weight:600}
.hi-system-error{border:1px solid var(--accent);padding:12px;border-radius:12px}
@container(max-width:560px){.hi-system-grid{grid-template-columns:1fr}.hi-system-row>div{width:100%;justify-content:space-between}}
@media(max-width:400px){.hi-system{padding-left:16px;padding-right:16px}}
`;
