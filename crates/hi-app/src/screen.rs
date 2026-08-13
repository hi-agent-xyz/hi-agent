//! The roster screen — who this app can be with, and who it is with now.
//!
//! See [`docs/arch/topology.md`](../../docs/arch/topology.md)`#app`.
//!
//! ## Why the app serves this, and not the core
//!
//! The roster is app state. A core has none, does not know other cores exist
//! ([invariant 4](../../docs/arch/topology.md)), and `_builtin/reach` leaves the
//! roster out deliberately — a view bundled into the core asking for it reaches
//! across the boundary this design draws, and 404s on every core with no app in
//! front of it.
//!
//! ## Why it is self-contained, and not part of the face
//!
//! **You need this screen exactly when no core is reachable.** Adding your first
//! one, or the attached one being asleep, are the two moments it exists for — and
//! the face is served *by the attached core*, so a roster living inside it is
//! unreachable precisely then. Same reasoning as the pairing page
//! (the core's `foundation::surfaces`), which is self-contained for the same kind
//! of reason: it is shown to a browser that may fetch nothing else yet.
//!
//! So: one file, no assets, no imports. It costs a few kilobytes and it works
//! when nothing else does.
//!
//! Its fetches are **root-relative**, unlike the pairing page's. The pairing page
//! is served by a core, which may be under a subpath; the app is always at the
//! root of its own loopback origin, so `/api/app/...` cannot be wrong here and
//! does not depend on whether the page was reached as `/app` or `/app/`.

/// `GET /app` — the roster, as a page.
///
/// Renders the list first and fills in reachability afterwards, one request per
/// entry, because a core that is off takes the full timeout to say so and a
/// person should not wait on the slowest one to see the list at all.
pub fn page() -> String {
    r##"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Who to be with</title>
<style>
  :root { color-scheme: light dark }
  body { font: 16px/1.5 -apple-system, system-ui, sans-serif; margin: 0;
         min-height: 100dvh; padding: 24px; display: grid; place-items: start center }
  main { width: min(34rem, 100%); display: grid; gap: 20px }
  h1 { font-size: 1.1rem; margin: 0 }
  h2 { font-size: .95rem; margin: 0; opacity: .7; font-weight: 600 }
  p  { margin: 0; opacity: .7; font-size: .9rem }
  ul { list-style: none; margin: 0; padding: 0; display: grid; gap: 8px }
  li { display: grid; grid-template-columns: auto 1fr auto auto; gap: 10px;
       align-items: center; padding: 12px 14px; border-radius: 12px;
       border: 1px solid color-mix(in srgb, currentColor 18%, transparent) }
  li.on { border-color: color-mix(in srgb, currentColor 55%, transparent);
          background: color-mix(in srgb, currentColor 6%, transparent) }
  .dot { width: 9px; height: 9px; border-radius: 50%; background: currentColor; opacity: .25 }
  .dot.here { background: #2ecc71; opacity: 1 }
  .dot.asleep { background: #e0a800; opacity: 1 }
  .dot.unreachable { background: #c0392b; opacity: 1 }
  .who { display: grid; gap: 2px; min-width: 0 }
  .label { font-weight: 600 }
  .addr { font-size: .8rem; opacity: .6; overflow: hidden; text-overflow: ellipsis; white-space: nowrap }
  .tag { font-size: .75rem; opacity: .6 }
  form { display: grid; gap: 10px }
  input, button { font: inherit; padding: 9px 12px; border-radius: 10px;
                  border: 1px solid color-mix(in srgb, currentColor 25%, transparent);
                  background: transparent; color: inherit }
  button { cursor: pointer }
  button.link { border: 0; padding: 4px 6px; opacity: .55; font-size: .85rem }
  button.link:hover { opacity: 1 }
  .bad { color: #c0392b; min-height: 1.4em; font-size: .9rem }
</style></head>
<body><main>
  <div>
    <h1>Who to be with</h1>
    <p>One at a time. Switching repoints this app; nothing about either agent changes.</p>
  </div>

  <ul id="list"></ul>
  <div class="bad" id="err"></div>

  <form id="add">
    <h2>Add a core</h2>
    <p>Its address, and a pairing code from a device that already has access.</p>
    <input id="base" placeholder="https://hi-agent.xyz/ana" autocomplete="off"
           autocapitalize="off" spellcheck="false">
    <input id="code" placeholder="pairing code" autocomplete="off"
           autocapitalize="off" spellcheck="false">
    <input id="label" placeholder="what to call it (optional)" autocomplete="off">
    <button type="submit">Add</button>
  </form>
</main>
<script>
const err = document.getElementById("err");
const list = document.getElementById("list");

function say(m) { err.textContent = m || ""; }

async function load() {
  say("");
  let roster = [];
  try {
    const res = await fetch("/api/app/roster");
    if (!res.ok) throw new Error(await res.text());
    roster = (await res.json()).roster || [];
  } catch (e) {
    say("Could not read the roster. " + e.message);
    return;
  }
  list.replaceChildren(...roster.map(render));
  // Reachability after the list is up: a core that is off takes the whole
  // timeout to say so, and one slow entry must not hold up the rest.
  roster.forEach(async (e) => {
    try {
      const res = await fetch("/api/app/roster/" + encodeURIComponent(e.id) + "/health");
      if (!res.ok) return;
      const { state } = await res.json();
      const dot = document.querySelector('[data-dot="' + CSS.escape(e.id) + '"]');
      if (dot) { dot.className = "dot " + state; dot.title = state; }
      const tag = document.querySelector('[data-tag="' + CSS.escape(e.id) + '"]');
      if (tag && state !== "here") tag.textContent = state;
    } catch (_) { /* a probe that fails is just an unknown dot */ }
  });
}

function render(e) {
  const li = document.createElement("li");
  if (e.attached) li.className = "on";

  const dot = document.createElement("span");
  dot.className = "dot";
  dot.dataset.dot = e.id;
  dot.title = "checking";

  const who = document.createElement("div");
  who.className = "who";
  const label = document.createElement("div");
  label.className = "label";
  label.textContent = e.label;
  const addr = document.createElement("div");
  addr.className = "addr";
  addr.textContent = e.base_url;
  who.append(label, addr);

  const tag = document.createElement("span");
  tag.className = "tag";
  tag.dataset.tag = e.id;
  tag.textContent = e.attached ? "with" : "";

  const act = document.createElement("div");
  if (!e.attached) {
    const be = document.createElement("button");
    be.className = "link";
    be.textContent = "be with";
    be.onclick = () => go("/api/app/roster/" + encodeURIComponent(e.id) + "/attach", "POST");
    act.append(be);
  }
  const forget = document.createElement("button");
  forget.className = "link";
  forget.textContent = "forget";
  forget.onclick = () => {
    // Forgetting is local. The credential stays live at the core until it is
    // revoked *there* — which is what lets losing a device be fixed without it.
    if (!confirm("Forget " + e.label + "? Its credential stays valid until you revoke it at that core.")) return;
    go("/api/app/roster/" + encodeURIComponent(e.id), "DELETE");
  };
  act.append(forget);

  li.append(dot, who, tag, act);
  return li;
}

async function go(url, method) {
  say("");
  const res = await fetch(url, { method, headers: { "X-HI-Surface": "1" } });
  if (!res.ok) { say(await res.text()); return; }
  load();
}

document.getElementById("add").addEventListener("submit", async (ev) => {
  ev.preventDefault();
  say("");
  const base_url = document.getElementById("base").value.trim();
  const code = document.getElementById("code").value.trim();
  const label = document.getElementById("label").value.trim();
  if (!base_url || !code) { say("An address and a pairing code, both."); return; }
  const res = await fetch("/api/app/roster", {
    method: "POST",
    headers: { "Content-Type": "application/json", "X-HI-Surface": "1" },
    body: JSON.stringify({ base_url, code, label }),
  });
  if (!res.ok) { say(await res.text()); return; }
  document.getElementById("base").value = "";
  document.getElementById("code").value = "";
  document.getElementById("label").value = "";
  load();
});

load();
</script>
</body></html>
"##
    .to_string()
}
