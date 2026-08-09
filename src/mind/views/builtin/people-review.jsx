// purpose: 认识的人 — review stored faces and voices, name the unknown ones, eject a mis-clustered clip, regroup a mixed cluster.
// A calm Contacts-style grid; clicking a
// card expands it in place (FLIP) into a review row — poster left, editable name +
// per-modality clip strips right/below. Naming onto an existing name merges. Every
// action posts to /api/people/*; the store is global.
import { useState, useEffect, useRef, useCallback, useLayoutEffect } from "react";

const api = {
  list: () => fetch("/api/people").then((r) => r.json()),
  name: (subject, name) =>
    fetch("/api/people/name", { method: "POST", headers: J, body: JSON.stringify({ subject, name }) }).then((r) => r.json()),
  eject: (subject, modality, stem) =>
    fetch("/api/people/eject", { method: "POST", headers: J, body: JSON.stringify({ subject, modality, stem }) }).then((r) => r.json()),
  preview: (subject, modality) =>
    fetch("/api/people/split/preview", { method: "POST", headers: J, body: JSON.stringify({ subject, modality }) }).then((r) => r.json()),
  applySplit: (subject, modality, groups) =>
    fetch("/api/people/split/apply", { method: "POST", headers: J, body: JSON.stringify({ subject, modality, groups }) }).then((r) => r.json()),
};
const J = { "Content-Type": "application/json" };
const clipUrl = (subject, modality, stem) =>
  `/api/people/${encodeURIComponent(subject)}/${modality}/${encodeURIComponent(stem)}`;

// ── words ─────────────────────────────────────────────────────────────────────
// English is the default and the fallback. This view is about people, not about this
// system's own vocabulary, so everything here is said in the reader's language.
// The two `lead` sentences return JSX because the named one bolds the person.
//
// TODO(i18n): en + zh are hand-written. Further languages are meant to be authored at
// runtime — the agent reads the surface and writes the variant — rather than shipped
// here. Until that exists, an unsupported language lands on English.
const T = {
  en: {
    title: "People",
    // On a fresh install this is the whole view. An empty grid under a title reads as a
    // page still loading; "no one yet" is a real answer and says what fills it.
    emptyBig: "No one stored yet.",
    emptySub: "As it meets people — a face on the camera, a voice in the room — they show up here as cards for you to name.",
    unnamed: "Unnamed", namePh: "Add a name…",
    mergeHint: (n) => `If there's already a "${n}", saving merges the two together`,
    sec: { face: "Faces", voice: "Voices" },
    noun: { face: "face", voice: "voice" },
    regroup: "Regroup automatically ⟳",
    oneOnly: "Looks like just one person — no need to split.", ok: "OK",
    notThisPerson: "Not this person",
    leadNamed: (name, noun) => <> Some of <b>{name}</b>'s {noun} clips look like someone else. The bigger pile stays {name}; take a look at the other one and tell me who it is.</>,
    leadUnnamed: (noun, n) => <>These {noun} clips look like more than one person, so I split them into {n} piles.</>,
    groupN: (i) => `Group ${i}`, keep: "Kept", countN: (n) => `${n} clips`,
    later: "Leave it", apply: "Split them",
  },
  zh: {
    title: "认识的人",
    emptyBig: "还没记住谁。",
    emptySub: "见过面、听过声音之后，人会一张张出现在这里，你来给他们起名字。",
    unnamed: "未命名", namePh: "加个名字…",
    mergeHint: (n) => `已经有「${n}」的话，保存会合并到一起`,
    sec: { face: "人脸", voice: "声音" },
    noun: { face: "脸", voice: "声音" },
    regroup: "自动重新分组 ⟳",
    oneOnly: "看起来就是一个人，没必要分。", ok: "好",
    notThisPerson: "不是这个人",
    leadNamed: (name, noun) => <> <b>{name}</b>的{noun}里像是混进了别人。大的一份还是 {name}，另一份挑出来你再认。</>,
    leadUnnamed: (noun, n) => <>这些{noun}像是不止一个人，我分成了 {n} 份。</>,
    groupN: (i) => `第 ${i} 组`, keep: "保留", countN: (n) => `${n} 个`,
    later: "先不动", apply: "就这样分",
  },
};

// App setting first — the host puts it on `<html lang>` — then the system locale when
// that setting says to follow the person, then English.
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

export default function PeopleReview() {
  const [people, setPeople] = useState(null);
  const [openId, setOpenId] = useState(null);
  const gridRef = useRef(null);
  const rects = useRef(new Map()); // FLIP: id -> DOMRect before a change

  const reload = useCallback(async () => {
    const d = await api.list().catch(() => ({ people: [] }));
    setPeople(d.people || []);
  }, []);
  useEffect(() => { reload(); }, [reload]);

  // FLIP: snapshot every card's rect just before a layout-affecting state change.
  const snapshot = () => {
    const m = new Map();
    gridRef.current?.querySelectorAll("[data-card]").forEach((el) => m.set(el.dataset.card, el.getBoundingClientRect()));
    rects.current = m;
  };
  // After the DOM updates, invert+play the delta so cards glide to their new spots.
  useLayoutEffect(() => {
    const first = rects.current;
    if (!first.size) return;
    gridRef.current?.querySelectorAll("[data-card]").forEach((el) => {
      const f = first.get(el.dataset.card);
      if (!f) return;
      const l = el.getBoundingClientRect();
      const dx = f.left - l.left, dy = f.top - l.top, sx = f.width / l.width, sy = f.height / l.height;
      if (Math.abs(dx) < 1 && Math.abs(dy) < 1 && Math.abs(sx - 1) < 0.01 && Math.abs(sy - 1) < 0.01) return;
      el.animate(
        [{ transformOrigin: "top left", transform: `translate(${dx}px,${dy}px) scale(${sx},${sy})` }, { transform: "none" }],
        { duration: 380, easing: "cubic-bezier(.32,.72,0,1)" },
      );
    });
    rects.current = new Map();
  });

  const open = (id) => { snapshot(); setOpenId(id); };
  const close = () => { snapshot(); setOpenId(null); };

  // Promote the open card to the start of its visual row so the panel spans a row and
  // the cards ahead of it backfill upward.
  const ordered = orderForOpen(people || [], openId, gridRef.current);

  // Click outside the open card collapses it.
  useEffect(() => {
    if (!openId) return;
    const onDown = (e) => {
      const openEl = gridRef.current?.querySelector("[data-open]");
      if (openEl && !openEl.contains(e.target) && !e.target.closest("[data-card]")) close();
    };
    document.addEventListener("pointerdown", onDown);
    return () => document.removeEventListener("pointerdown", onDown);
  }, [openId]);

  if (people === null) return <div style={S.page}><div style={S.h1}>{L.title}</div></div>;

  if (people.length === 0) {
    return (
      <div style={S.page}>
        <div style={S.h1}>{L.title}</div>
        <div style={S.empty}>
          <div style={S.emptyBig}>{L.emptyBig}</div>
          <div style={S.emptySub}>{L.emptySub}</div>
        </div>
      </div>
    );
  }

  return (
    <div style={S.page}>
      <style>{"@keyframes hi-ppl-eqpulse{0%,100%{height:10px}50%{height:26px}}"}</style>
      <div style={S.h1}>{L.title}</div>
      <div style={S.grid} ref={gridRef}>
        {ordered.map((p) =>
          p.subject === openId ? (
            <Review key={p.subject} person={p} onClose={close} onChanged={reload} />
          ) : (
            <Card key={p.subject} person={p} onOpen={() => open(p.subject)} />
          ),
        )}
      </div>
    </div>
  );
}

function Card({ person, onOpen }) {
  const isFace = person.face.length > 0;
  const poster = isFace ? clipUrl(person.subject, "face", person.face[0]) : null;
  return (
    <button type="button" data-card={person.subject} style={{ ...S.reset, ...S.card }} onClick={onOpen}
      onMouseEnter={(e) => lift(e.currentTarget, true)} onMouseLeave={(e) => lift(e.currentTarget, false)}>
      {poster ? (
        <div style={{ ...S.poster, backgroundImage: `url('${poster}')` }} />
      ) : (
        <div style={{ ...S.poster, ...S.voicePoster }}><Eq /></div>
      )}
      <div style={person.named ? S.name : S.nameNone}>{person.named ? person.subject : L.unnamed}</div>
    </button>
  );
}

function Review({ person, onClose, onChanged }) {
  const [name, setName] = useState(person.named ? person.subject : "");
  const [merge, setMerge] = useState("");
  const isFace = person.face.length > 0;

  const save = async () => {
    const v = name.trim();
    if (!v || v === (person.named ? person.subject : "")) return;
    await api.name(person.subject, v);
    onChanged();
  };

  return (
    <div data-card={person.subject} data-open style={S.review}>
      <div style={S.revHead}>
        {isFace ? (
          <div style={{ ...S.revPoster, backgroundImage: `url('${clipUrl(person.subject, "face", person.face[0])}')` }} />
        ) : (
          <div style={{ ...S.revPoster, ...S.voicePoster }}><Eq big /></div>
        )}
        <div style={S.revMeta}>
          <input
            style={S.nameInput}
            value={name}
            placeholder={L.namePh}
            onChange={(e) => setName(e.target.value)}
            onInput={(e) => setMerge(e.target.value.trim())}
            onBlur={save}
            onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
          />
          <div style={S.mergeHint}>{merge && merge !== person.subject ? L.mergeHint(merge) : ""}</div>
        </div>
        <button style={S.close} onClick={onClose}>✕</button>
      </div>
      <div style={S.revBody}>
        {person.face.length > 0 && <ModSection person={person} modality="face" onChanged={onChanged} first />}
        {person.voice.length > 0 && <ModSection person={person} modality="voice" onChanged={onChanged} first={person.face.length === 0} />}
      </div>
    </div>
  );
}

function ModSection({ person, modality, onChanged, first }) {
  const stems = modality === "face" ? person.face : person.voice;
  const [proposal, setProposal] = useState(null);
  const messy = stems.length >= 4 || person.recurring;

  const regroup = async () => {
    const p = await api.preview(person.subject, modality);
    setProposal(p.groups && p.groups.length >= 2 ? p : { none: true });
  };

  return (
    <div style={{ ...S.modsec, ...(first ? {} : S.modsecTop) }}>
      <div style={S.secttl}>
        <span>{L.sec[modality]} <span style={S.cnt}>{stems.length}</span></span>
        {messy && <button type="button" style={{ ...S.reset, ...S.regroup }} onClick={regroup}>{L.regroup}</button>}
      </div>
      <div style={S.clips}>
        {stems.map((stem) => (
          <Clip key={stem} subject={person.subject} modality={modality} stem={stem} onChanged={onChanged} />
        ))}
      </div>
      {proposal && !proposal.none && (
        <Proposal person={person} modality={modality} proposal={proposal} onClose={() => setProposal(null)} onChanged={onChanged} />
      )}
      {proposal && proposal.none && (
        <div style={S.plead}>
          {L.oneOnly}
          <button type="button" style={{ ...S.reset, ...S.link }} onClick={() => setProposal(null)}>{L.ok}</button>
        </div>
      )}
    </div>
  );
}

function Clip({ subject, modality, stem, onChanged }) {
  const [playing, setPlaying] = useState(false);
  const [gone, setGone] = useState(false);
  const audioRef = useRef(null);
  const url = clipUrl(subject, modality, stem);

  const isVoice = modality === "voice";
  const play = (e) => {
    e.stopPropagation();
    if (!isVoice) return;
    if (!audioRef.current) audioRef.current = new Audio(url);
    const a = audioRef.current;
    if (playing) { a.pause(); setPlaying(false); }
    else { a.currentTime = 0; a.play().catch(() => {}); setPlaying(true); a.onended = () => setPlaying(false); }
  };
  const eject = async (e) => {
    e.stopPropagation();
    setGone(true);
    setTimeout(async () => { await api.eject(subject, modality, stem); onChanged(); }, 300);
  };

  const base = isVoice
    ? { ...S.clip, ...S.voiceClip }
    : { ...S.clip, backgroundImage: `url('${url}')`, backgroundSize: "cover", backgroundPosition: "center",
        cursor: "default" };

  // Only a voice clip plays, so only a voice clip offers the play glyph, the pointer and
  // a key handler — a ▶ on a face crop promised something no click could deliver. The
  // tile stays a div because it already contains the eject button.
  return (
    <div style={{ ...base, ...(gone ? S.clipGone : {}), ...(playing ? S.clipPlaying : {}) }}
      onClick={play}
      role={isVoice ? "button" : undefined}
      tabIndex={isVoice ? 0 : undefined}
      aria-pressed={isVoice ? playing : undefined}
      onKeyDown={isVoice ? (e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); play(e); } } : undefined}>
      {isVoice && <Eq small live={playing} />}
      <button type="button" style={S.eject} title={L.notThisPerson} aria-label={L.notThisPerson} onClick={eject}>✕</button>
      {isVoice && <div style={S.clipPlay}>▶</div>}
    </div>
  );
}

function Proposal({ person, modality, proposal, onClose, onChanged }) {
  const groups = proposal.groups;
  // Largest group keeps the name; render it flagged.
  let keepIdx = 0;
  groups.forEach((g, i) => { if (g.stems.length > groups[keepIdx].stems.length) keepIdx = i; });
  const apply = async () => {
    await api.applySplit(person.subject, modality, groups.map((g) => g.stems));
    onChanged();
  };
  return (
    <div style={S.proposal}>
      <div style={S.plead}>
        {person.named
          ? L.leadNamed(person.subject, L.noun[modality])
          : L.leadUnnamed(L.noun[modality], groups.length)}
      </div>
      <div style={S.piles}>
        {groups.map((g, i) => (
          <div key={i} style={{ ...S.pile, ...(i === keepIdx ? S.pileKeep : {}) }}>
            <div style={S.pileHead}>
              <span>{i === keepIdx && person.named ? person.subject : L.groupN(i + 1)}</span>
              {i === keepIdx && <span style={S.keepTag}>{L.keep}</span>}
            </div>
            <div style={S.pileThumbs}>
              {g.stems.slice(0, 4).map((stem) =>
                modality === "face"
                  ? <div key={stem} style={{ ...S.pt, backgroundImage: `url('${clipUrl(person.subject, "face", stem)}')` }} />
                  : <div key={stem} style={{ ...S.pt, ...S.voiceClip, fontSize: 13 }}>♪</div>,
              )}
            </div>
            <div style={S.pileCnt}>{L.countN(g.stems.length)}</div>
          </div>
        ))}
      </div>
      <div style={S.pbtns}>
        <button style={S.btnGhost} onClick={onClose}>{L.later}</button>
        <button style={S.btnPrimary} onClick={apply}>{L.apply}</button>
      </div>
    </div>
  );
}

function Eq({ big, small, live }) {
  const n = 6;
  const w = big ? 5 : small ? 2.5 : 4;
  const h = big ? 22 : small ? 11 : 16;
  return (
    <div style={{ display: "flex", gap: small ? 2.5 : 4, alignItems: "center" }}>
      {Array.from({ length: n }).map((_, i) => (
        <i key={i} style={{
          width: w, height: h, borderRadius: w, display: "inline-block",
          background: live ? "var(--accent)" : "var(--fg-mute)", opacity: live ? 0.9 : 0.5,
          // Prefixed: the keyframes go into the host document, shared with every other
          // view on the page, so a bare `eqpulse` is a name collision waiting to happen.
          animation: live ? `hi-ppl-eqpulse 1s ease-in-out ${i * 0.1}s infinite` : "none",
        }} />
      ))}
    </div>
  );
}

// FLIP helper: move the open card to the first slot of its visual row.
function orderForOpen(people, openId, gridEl) {
  if (!openId) return people;
  const idx = people.findIndex((p) => p.subject === openId);
  if (idx < 0) return people;
  const cols = columnsOf(gridEl);
  const rowStart = Math.floor(idx / cols) * cols;
  const arr = people.slice();
  const [openC] = arr.splice(idx, 1);
  arr.splice(rowStart, 0, openC);
  return arr;
}
function columnsOf(gridEl) {
  if (!gridEl) return 1;
  const cols = getComputedStyle(gridEl).gridTemplateColumns.split(" ").filter(Boolean).length;
  return Math.max(1, cols);
}
function lift(el, on) {
  el.style.transform = on ? "translateY(-3px)" : "none";
  el.style.boxShadow = on ? "var(--ppl-shadow-lift)" : "var(--ppl-shadow)";
}

// Inline styles keyed off the host's theme tokens, so the view rides light/dark with
// the person's Theme setting. The vocabulary is what `ui/global.css` actually defines:
//
//   text      --fg · --fg-dim · --fg-mute
//   surfaces  --surface · --surface-strong   (cards/panels over the paper)
//   lines     --line · --line-strong         (borders, and neutral placeholder fills)
//   accent    --accent · --accent-soft · --accent-line · --accent-wash
//   shadow    --shadow · --shadow-strong     (COLOURS, not box-shadow lists)
//   type      --font-display
//
// Two traps this file used to fall into, both worth keeping in mind when copying it:
//   1. Don't invent token names. `--card` / `--page` / `--bg` / `--eqbar` were never
//      defined anywhere, so `var(--card,#fff)` silently pinned the card to white while
//      `--fg` went on flipping — which is how named people turned invisible in dark.
//   2. Don't shadow a host token. This block used to define its own `--shadow`, which
//      is a host colour token; the local names are prefixed now.
const S = {
  page: { "--ppl-shadow": "0 1px 2px var(--shadow),0 8px 22px var(--shadow)",
    "--ppl-shadow-lift": "0 4px 10px var(--shadow),0 22px 55px var(--shadow-strong)",
    width: "100%", height: "100%", minHeight: 0, overflowY: "auto", boxSizing: "border-box",
    padding: "28px clamp(20px,3vw,44px) 128px", background: "var(--bg-0)",
    color: "var(--fg)", fontFamily: "var(--font-display)" },
  h1: { fontSize: 30, fontWeight: 800, letterSpacing: 0, marginBottom: 26 },
  // Everything that responds to a click is a real <button>, so it is reachable by tab
  // and by Enter/Space for free. This strips the UA chrome back to the div it replaced.
  reset: { appearance: "none", border: "none", background: "none", font: "inherit",
    color: "inherit", textAlign: "left", cursor: "pointer", padding: 0 },

  empty: { padding: "46px 8px", textAlign: "center" },
  emptyBig: { fontSize: 17, fontWeight: 600, color: "var(--fg-dim)" },
  emptySub: { fontSize: 13.5, color: "var(--fg-mute)", marginTop: 7, lineHeight: 1.55,
    maxWidth: 460, marginLeft: "auto", marginRight: "auto" },
  grid: { display: "grid", gridTemplateColumns: "repeat(auto-fill,minmax(168px,1fr))", gap: 18, alignItems: "start" },
  // `display:block` + full width: as a <button> this would otherwise shrink-wrap its
  // contents and centre them, instead of filling its grid cell the way the div did.
  card: { display: "block", width: "100%", background: "var(--surface-strong)", borderRadius: 22,
    boxShadow: "var(--ppl-shadow)", overflow: "hidden",
    cursor: "pointer", transition: "transform .2s cubic-bezier(.32,.72,0,1),box-shadow .2s" },
  // `backgroundColor`, never the `background` shorthand: the shorthand resets
  // background-size/position, which left every face crop tiling at natural size.
  poster: { aspectRatio: "1/1", backgroundSize: "cover", backgroundPosition: "center",
    backgroundRepeat: "no-repeat", backgroundColor: "var(--line-strong)" },
  voicePoster: { display: "flex", alignItems: "center", justifyContent: "center",
    backgroundImage: "linear-gradient(150deg,var(--line),var(--line-strong))" },
  name: { padding: "13px 15px 15px", fontSize: 15, fontWeight: 700, letterSpacing: "-.02em" },
  nameNone: { padding: "13px 15px 15px", fontSize: 15, fontWeight: 500, color: "var(--fg-mute)" },

  review: { gridColumn: "1 / -1", background: "var(--surface-strong)", borderRadius: 28,
    boxShadow: "var(--ppl-shadow-lift)", overflow: "hidden" },
  revHead: { display: "flex", gap: 22, padding: "26px 28px 22px", alignItems: "center", borderBottom: "1px solid var(--line)" },
  revPoster: { width: 108, height: 108, borderRadius: 24, backgroundSize: "cover", backgroundPosition: "center",
    backgroundRepeat: "no-repeat", flex: "none", backgroundColor: "var(--line-strong)" },
  revMeta: { flex: 1, minWidth: 0 },
  nameInput: { font: "inherit", fontSize: 27, fontWeight: 800, letterSpacing: "-.03em", color: "var(--fg)",
    background: "transparent", border: "none", outline: "none", width: "100%", padding: "2px 0", borderBottom: "2px solid transparent" },
  mergeHint: { fontSize: 13, color: "var(--accent)", marginTop: 8, minHeight: 17, fontWeight: 500 },
  close: { flex: "none", width: 34, height: 34, borderRadius: "50%", border: "none", background: "var(--line)",
    color: "var(--fg)", fontSize: 16, cursor: "pointer", alignSelf: "flex-start" },
  revBody: { padding: "4px 28px 28px" },
  modsec: { paddingTop: 18 },
  modsecTop: { borderTop: "1px solid var(--line)", marginTop: 8 },
  secttl: { fontSize: 14, fontWeight: 700, margin: "14px 0", display: "flex", alignItems: "center", justifyContent: "space-between" },
  cnt: { color: "var(--fg-mute)", fontWeight: 500, marginLeft: 6 },
  regroup: { fontSize: 12.5, fontWeight: 600, color: "var(--fg-dim)", cursor: "pointer", padding: "6px 12px", borderRadius: 999 },
  clips: { display: "grid", gridTemplateColumns: "repeat(auto-fill,minmax(86px,1fr))", gap: 10 },
  clip: { position: "relative", borderRadius: 13, overflow: "hidden", backgroundColor: "var(--line-strong)", aspectRatio: "1/1",
    cursor: "pointer", display: "flex", alignItems: "center", justifyContent: "center",
    transition: "transform .18s,opacity .3s" },
  voiceClip: { backgroundImage: "linear-gradient(150deg,var(--line),var(--line-strong))" },
  clipGone: { opacity: 0, transform: "scale(.6)" },
  clipPlaying: { boxShadow: "0 0 0 2px var(--accent) inset" },
  clipPlay: { position: "absolute", right: 6, bottom: 5, fontSize: 12, color: "#fff",
    textShadow: "0 1px 3px rgba(0,0,0,.6)", opacity: 0.9, pointerEvents: "none" },
  // Literal dark scrim + white glyph: these sit on top of a photo, not on the surface,
  // so they take the same always-legible treatment the host gives caption pills.
  eject: { position: "absolute", top: 5, right: 5, width: 22, height: 22, borderRadius: "50%", border: "none",
    background: "rgba(0,0,0,.5)", color: "#fff", fontSize: 11, cursor: "pointer", display: "flex",
    alignItems: "center", justifyContent: "center", zIndex: 2 },

  proposal: { marginTop: 14, background: "var(--accent-wash)", borderRadius: 20, padding: "20px 22px" },
  plead: { fontSize: 14, color: "var(--fg)", opacity: 0.75, marginBottom: 16, lineHeight: 1.5 },
  link: { color: "var(--accent)", cursor: "pointer", marginLeft: 6, fontWeight: 600 },
  piles: { display: "flex", gap: 14, flexWrap: "wrap" },
  pile: { flex: 1, minWidth: 170, background: "var(--surface-strong)", borderRadius: 16, padding: 15, boxShadow: "var(--ppl-shadow)" },
  pileKeep: { outline: "2px solid var(--accent)" },
  pileHead: { display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 11, fontSize: 14, fontWeight: 700 },
  keepTag: { fontSize: 11, fontWeight: 700, color: "var(--accent)" },
  pileThumbs: { display: "flex", gap: 5, flexWrap: "wrap", marginBottom: 10 },
  pt: { width: 38, height: 38, borderRadius: 9, backgroundSize: "cover", backgroundPosition: "center",
    backgroundRepeat: "no-repeat", backgroundColor: "var(--line-strong)",
    display: "flex", alignItems: "center", justifyContent: "center", color: "var(--fg-mute)" },
  pileCnt: { fontSize: 12, color: "var(--fg-mute)" },
  pbtns: { display: "flex", gap: 11, marginTop: 18 },
  btnGhost: { padding: "11px 20px", borderRadius: 13, fontWeight: 700, fontSize: 14, cursor: "pointer", border: "none",
    background: "var(--surface-strong)", color: "var(--fg)" },
  // Label takes the page ground, not a literal white: on the dark theme's amber accent
  // white lands at 2.4:1, while the ground colour gives 6.8:1.
  btnPrimary: { padding: "11px 20px", borderRadius: 13, fontWeight: 700, fontSize: 14, cursor: "pointer", border: "none",
    background: "var(--accent)", color: "var(--bg-0)" },
};
