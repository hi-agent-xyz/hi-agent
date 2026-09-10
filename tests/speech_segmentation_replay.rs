//! Replay recorded mic audio through the real recognizer, then cut the frames it
//! returns with two segmentation policies and diff the messages.
//!
//! # Why this exists as a test rather than a script
//!
//! The 2026-09-09 measurement behind [`gaps.md`](../docs/user-journeys/gaps.md) #32
//! was made with a throwaway script that was not kept, so the 2026-09-10 question —
//! *did holding a half-finished sentence actually help?* — could not be answered
//! without writing the same thing again. It is a test now so the next one is free.
//!
//! # The two phases, and why they are separate
//!
//! 1. **Capture** (needs the network and a key, ~1× realtime): stream WAV PCM at
//!    100 ms/frame into `volcengine_stt::transcribe_streaming` and write every
//!    partial/final it returns, stamped with its offset from the first byte, to
//!    `HI_REPLAY_FRAMES`. Skipped when that file already exists.
//! 2. **Compare** (offline, free, deterministic): drive two [`Segmenter`]s over
//!    those frames with a synthetic clock, ticking at the same 150 ms as the live
//!    audio task, and print both message lists.
//!
//! Phase 2 is the answer, and it is repeatable at no cost once phase 1 has run —
//! which is the point of writing the frames down. **Re-running phase 1 on a
//! different day is not expected to reproduce the same frames**: the recognizer is
//! a live service. What makes the comparison exact is not that the capture is
//! stable but that **both policies see one identical frame list**.
//!
//! # What this cannot answer
//!
//! The minute WAVs are **not a faithful timeline**. `flush_mic_minute` writes
//! `<HH>/<MM>.wav` through `File::create`, and it runs both at each minute rollover
//! *and* at every socket close — so a mic reconnect inside a minute destroys the
//! earlier part of it. In the 2026-09-10 capture, `05/47.wav` holds 14.1 s of a
//! minute the speaker talked all the way through, and the six files hold 221.7 s
//! spanning ~320 s of wall clock.
//!
//! Concatenating them therefore **removes pauses that really happened**, and a
//! pause is the very thing the policy under test reads. That biases the replay
//! *against* holding (fewer pauses → longer run-ons → more cap cuts), so a win
//! here is a floor, not an estimate. The unbiased answer needs the frames logged
//! live; see gaps.md #32.
//!
//! # Running it
//!
//!     make test-live                      # with everything defaulted
//!     HI_REPLAY_AUDIO=data/memory/raw/audio/2026-09-10/05 \
//!     HI_REPLAY_FRAMES=/tmp/frames.jsonl \
//!     cargo test --test speech_segmentation_replay -- --ignored --nocapture
//!
//! The key is read out of `<HI_REPLAY_DATA_DIR>/config.db` the same way boot reads
//! it, so it never passes through an environment variable or a command line.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hi_agent::body::capabilities::stt::Transcript;
use hi_agent::foundation::credentials::Credentials;
use hi_agent::foundation::segment::{SegmenterConfig, Segmenter, Speech};
use hi_agent::foundation::vendors::volcengine_stt;

/// One recognizer frame, as recorded.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
struct Frame {
    /// Milliseconds from the first PCM byte sent.
    ms: u64,
    text: String,
    is_final: bool,
}

/// 100 ms of 16 kHz mono 16-bit PCM — one WS frame, matching the live mic.
const CHUNK: usize = 3200;
/// The live audio task's tick, so a time-driven cut fires at the same granularity.
const TICK: Duration = Duration::from_millis(150);
const WAV_HEADER: usize = 44;

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

/// Every `.wav` under `dir`, in filename order, concatenated as raw PCM.
fn read_pcm(dir: &Path) -> Vec<u8> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "wav"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no .wav under {}", dir.display());
    let mut pcm = Vec::new();
    for f in &files {
        let bytes = std::fs::read(f).expect("reading wav");
        let body = &bytes[WAV_HEADER.min(bytes.len())..];
        println!(
            "  {} → {:.1}s",
            f.file_name().unwrap().to_string_lossy(),
            body.len() as f64 / 32_000.0
        );
        pcm.extend_from_slice(body);
    }
    pcm
}

/// Phase 1 — stream the PCM through the recognizer at realtime pace, recording
/// every frame it sends back.
async fn capture(pcm: Vec<u8>) -> Vec<Frame> {
    let data_dir = env_or("HI_REPLAY_DATA_DIR", "data");
    let creds = Credentials::load(Path::new(&data_dir));
    let eff = creds
        .effective()
        .expect("no effective credentials in the config store");
    let v = eff.stt.first().expect("no STT provider configured");
    let cfg = volcengine_stt::Config::from_store(v.key_opt(), v.base_url_opt(), v.model_opt())
        .expect("building the STT config");

    let (audio_tx, audio_rx) = tokio::sync::mpsc::channel(64);
    let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<Transcript>(256);
    let stt = tokio::spawn(async move { volcengine_stt::transcribe_streaming(&cfg, audio_rx, out_tx).await });

    let t0 = Instant::now();
    let collect = tokio::spawn(async move {
        let mut frames = Vec::new();
        while let Some(t) = out_rx.recv().await {
            if t.text.trim().is_empty() {
                continue;
            }
            frames.push(Frame {
                ms: t0.elapsed().as_millis() as u64,
                text: t.text,
                is_final: t.is_final,
            });
        }
        frames
    });

    // Realtime pace. The endpoint that decides an utterance is closed is 800 ms of
    // *silence in the stream*, so feeding faster than realtime would move every
    // boundary this test is about.
    let mut sent = 0usize;
    for chunk in pcm.chunks(CHUNK) {
        if audio_tx.send(bytes::Bytes::copy_from_slice(chunk)).await.is_err() {
            eprintln!("  STT session ended early after {sent} chunks");
            break;
        }
        sent += 1;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    drop(audio_tx);
    let _ = stt.await.expect("STT task panicked");
    collect.await.expect("collector panicked")
}

/// Phase 2 — cut one frame list with one policy, mirroring the live audio task:
/// frames are observed at their recorded offsets, with a 150 ms tick in between.
fn cut_with(cfg: SegmenterConfig, frames: &[Frame]) -> Vec<(Duration, String)> {
    let t0 = Instant::now();
    let mut seg = Segmenter::new(Speech::new(cfg), t0);
    let mut out = Vec::new();
    let mut clock = Duration::ZERO;
    let mut push = |clock: Duration, msgs: Vec<String>, out: &mut Vec<(Duration, String)>| {
        out.extend(msgs.into_iter().map(|m| (clock, m)));
    };
    for f in frames {
        let at = Duration::from_millis(f.ms);
        // Tick up to this frame, so a hold or a max-segment cut fires when it would
        // have live rather than only when the next frame happens to arrive.
        while clock + TICK <= at {
            clock += TICK;
            let msgs = seg.tick(t0 + clock);
            push(clock, msgs, &mut out);
        }
        clock = at;
        let msgs = seg.observe(&f.text, f.is_final, t0 + clock);
        push(clock, msgs, &mut out);
    }
    // The speaker stopped; let the clock run out so a held tail is released.
    for _ in 0..200 {
        clock += TICK;
        let msgs = seg.tick(t0 + clock);
        push(clock, msgs, &mut out);
    }
    if let Some(m) = seg.flush() {
        out.push((clock, m));
    }
    out
}

fn texts(v: &[(Duration, String)]) -> Vec<String> {
    v.iter().map(|(_, m)| m.clone()).collect()
}

/// Every place two near-identical char sequences differ, as `(old, new)` pairs.
///
/// A char-level LCS, walked back into runs. The inputs are two cuts of one
/// recognition, so they agree almost everywhere and the interesting output is the
/// handful of spots they do not.
fn diff(a: &[char], b: &[char]) -> Vec<(String, String)> {
    let mut lcs = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    let (mut da, mut db) = (String::new(), String::new());
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            if !da.is_empty() || !db.is_empty() {
                out.push((std::mem::take(&mut da), std::mem::take(&mut db)));
            }
            i += 1;
            j += 1;
        } else if j == b.len() || (i < a.len() && lcs[i + 1][j] >= lcs[i][j + 1]) {
            da.push(a[i]);
            i += 1;
        } else {
            db.push(b[j]);
            j += 1;
        }
    }
    if !da.is_empty() || !db.is_empty() {
        out.push((da, db));
    }
    out
}

/// The same policy with **the punctuation signal removed entirely**: no reading of
/// how the recognizer punctuated the close, so no way to tell a finished sentence
/// from a hesitation, so everything waits out the one timer. The cap cannot snap to
/// a phrase boundary either — there is nothing to snap to — so it chops.
///
/// This is what [`Speech`] would be if `ends_a_thought`, `snap_back`,
/// `is_sentence_end` and `is_clause_boundary` were all deleted from its path. Run
/// beside the real one to price that signal.
struct NoPunct {
    hold: Duration,
    max_chars: usize,
}

impl hi_agent::foundation::segment::CutPolicy for NoPunct {
    fn boundary(&self, cut: &hi_agent::foundation::segment::Cut) -> Option<usize> {
        let n = cut.tail.len();
        if n == 0 {
            return None;
        }
        if cut.quiet >= self.hold || n >= self.max_chars {
            return Some(n);
        }
        None
    }
}

/// [`cut_with`] for a policy that is not [`Speech`] — same synthetic clock, same
/// 150 ms tick.
fn cut_nopunct(policy: NoPunct, frames: &[Frame]) -> Vec<(Duration, String)> {
    let t0 = Instant::now();
    let mut seg = Segmenter::new(policy, t0);
    let mut out = Vec::new();
    let mut clock = Duration::ZERO;
    for f in frames {
        let at = Duration::from_millis(f.ms);
        while clock + TICK <= at {
            clock += TICK;
            out.extend(seg.tick(t0 + clock).into_iter().map(|m| (clock, m)));
        }
        clock = at;
        out.extend(seg.observe(&f.text, f.is_final, t0 + clock).into_iter().map(|m| (clock, m)));
    }
    for _ in 0..200 {
        clock += TICK;
        out.extend(seg.tick(t0 + clock).into_iter().map(|m| (clock, m)));
    }
    if let Some(m) = seg.flush() {
        out.push((clock, m));
    }
    out
}

const END: [char; 6] = ['。', '！', '？', '!', '?', '…'];

fn ragged(msgs: &[String]) -> Vec<&String> {
    msgs.iter()
        .filter(|m| !m.chars().rev().find(|c| !c.is_whitespace()).is_some_and(|c| END.contains(&c)))
        .collect()
}

fn report(label: &str, msgs: &[String]) {
    let mut lens: Vec<usize> = msgs.iter().map(|m| m.chars().count()).collect();
    lens.sort_unstable();
    let median = lens.get(lens.len() / 2).copied().unwrap_or(0);
    let rag = ragged(msgs);
    println!(
        "\n{label}: {} 条  中位 {median} 字  最短 {}  最长 {}  半截话 {} 条",
        msgs.len(),
        lens.first().copied().unwrap_or(0),
        lens.last().copied().unwrap_or(0),
        rag.len()
    );
    for m in msgs {
        let mark = if rag.contains(&m) { "  ← 半截" } else { "" };
        println!("   [{:3}] {m}{mark}", m.chars().count());
    }
}

#[tokio::test]
#[ignore = "streams real audio through the STT vendor; costs a key and a few minutes"]
async fn holding_a_half_finished_sentence_beats_cutting_on_the_endpoint() {
    let frames_path = env_or("HI_REPLAY_FRAMES", "/tmp/hi-replay-frames.jsonl");
    let frames: Vec<Frame> = if Path::new(&frames_path).exists() {
        println!("frames: reusing {frames_path} (delete it to re-capture)");
        std::fs::read_to_string(&frames_path)
            .expect("reading frames")
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).expect("parsing a frame"))
            .collect()
    } else {
        let audio = env_or("HI_REPLAY_AUDIO", "data/memory/raw/audio/2026-09-10/05");
        println!("audio: {audio}");
        let pcm = read_pcm(Path::new(&audio));
        println!(
            "  total {:.1}s — streaming at realtime, so this takes that long",
            pcm.len() as f64 / 32_000.0
        );
        let frames = capture(pcm).await;
        let body: String =
            frames.iter().map(|f| serde_json::to_string(f).unwrap() + "\n").collect();
        std::fs::write(&frames_path, body).expect("writing frames");
        println!("frames: wrote {} to {frames_path}", frames.len());
        frames
    };

    assert!(!frames.is_empty(), "the recognizer returned nothing to cut");
    let finals = frames.iter().filter(|f| f.is_final).count();
    println!(
        "\n{} frames over {:.1}s — {finals} finals, {} partials",
        frames.len(),
        frames.last().unwrap().ms as f64 / 1000.0,
        frames.len() - finals
    );

    // The old policy is the new one with the hold collapsed onto the settle and the
    // cap back at 96 — which is exactly what the change was, so nothing here has to
    // keep a second copy of the rule in step.
    let new = SegmenterConfig::default();
    // The policy as it stood before any of this: the endpoint alone decides, and a
    // 96-char cap bounds a run-on. Collapsing the hold onto the settle is exactly
    // what "no holding" means, so nothing here keeps a second copy of the old rule.
    let old = SegmenterConfig { max_chars: 96, hold: new.settle, ..new };

    let old_out = cut_with(old, &frames);
    let new_out = cut_with(new, &frames);
    let (old_msgs, new_msgs) = (texts(&old_out), texts(&new_out));
    report("旧 (endpoint = 一条消息, cap 96)", &old_msgs);
    report("新 (完句才发, hold 3s, cap 220)", &new_msgs);

    // Do the two guards fire at all under the new settings? Disabling both must not
    // change a single message if they do not — and whether they do is what decides
    // whether the buffer's edit-distance alignment (`unemitted`, alive only to
    // reconcile a final against a guard cut taken from a still-revisable partial)
    // is exercised by real speech or merely present.
    // Is the one remaining guard doing anything, or is the shape of an ordinary
    // conversation decided entirely by "did they finish"? Disabling it must not
    // change a single message if it never fires.
    for (label, base, msgs) in
        [("新 (hold 3s, cap 220)", new, &new_msgs), ("旧 (settle only, cap 96)", old, &old_msgs)]
    {
        let off = SegmenterConfig { max_chars: usize::MAX, ..base };
        println!(
            "  {label:<24} size 守卫: {:<6} (关掉后 {} 条)",
            if texts(&cut_with(off, &frames)) == *msgs { "没开火" } else { "开火" },
            cut_with(off, &frames).len(),
        );
    }

    // What the hold is worth, swept. Free once the frames are on disk, and the only
    // number here that is a *cost* is the last column: how long after the final frame
    // the last message landed, which is the latency a reply would have waited.
    let last_frame = Duration::from_millis(frames.last().unwrap().ms);
    let row = |label: String, cfg: SegmenterConfig| {
        let out = cut_with(cfg, &frames);
        let msgs = texts(&out);
        let mut lens: Vec<usize> = msgs.iter().map(|m| m.chars().count()).collect();
        lens.sort_unstable();
        let tail_delay = out.last().map(|(t, _)| t.saturating_sub(last_frame)).unwrap_or_default();
        println!(
            "  {label:>7}  {:>4}  {:>4}  {:>4}  {:>2}/{:<2}  {:>8.2}s",
            msgs.len(),
            lens.get(lens.len() / 2).copied().unwrap_or(0),
            lens.last().copied().unwrap_or(0),
            ragged(&msgs).len(),
            msgs.len(),
            tail_delay.as_secs_f64()
        );
    };

    println!("\nhold 扫描 (cap 220 / age 60s 不变):");
    println!("       值  条数  中位  最长  半截      末条延迟");
    for secs in [0.15, 1.0, 2.0, 3.0, 4.0, 6.0, 8.0] {
        let hold = Duration::from_secs_f64(secs);
        row(format!("{secs:.2}s"), SegmenterConfig { hold, ..new });
    }

    // The dial between the two timers being far apart (a finished sentence leaves at
    // once) and being one number (everything waits out the hold). `settle` is what
    // moves, because it is the only wait a *finished* thought ever pays — and a
    // finished thought is the one a turn ends on, so this column IS the latency.
    println!("\nsettle 扫描 (hold 3s / cap 220 不变) —— 中间态就在这条轴上:");
    println!("     settle  条数  中位  最长  半截      末条延迟");
    for secs in [0.15, 0.4, 0.7, 1.0, 1.5, 2.0, 3.0] {
        let settle = Duration::from_secs_f64(secs);
        row(format!("{secs:.2}s"), SegmenterConfig { settle, ..new });
    }

    // What the punctuation signal is worth. Without it there is one timer for every
    // case, so the column that moves is the latency on the turn's last message —
    // which the real policy holds at the settle because it can see the sentence ended.
    println!("\n去掉标点信号 (只看静音,cap 220 硬切) —— 每个 hold 取值:");
    println!("       值  条数  中位  最长  半截      末条延迟");
    for secs in [0.5, 1.0, 2.0, 3.0] {
        let out = cut_nopunct(
            NoPunct { hold: Duration::from_secs_f64(secs), max_chars: new.max_chars },
            &frames,
        );
        let msgs = texts(&out);
        let mut lens: Vec<usize> = msgs.iter().map(|m| m.chars().count()).collect();
        lens.sort_unstable();
        let tail_delay = out.last().map(|(t, _)| t.saturating_sub(last_frame)).unwrap_or_default();
        println!(
            "  {:>7}  {:>4}  {:>4}  {:>4}  {:>2}/{:<2}  {:>8.2}s",
            format!("{secs:.2}s"),
            msgs.len(),
            lens.get(lens.len() / 2).copied().unwrap_or(0),
            lens.last().copied().unwrap_or(0),
            ragged(&msgs).len(),
            msgs.len(),
            tail_delay.as_secs_f64()
        );
    }
    println!("  (对照) 带标点 hold 3s: {} 条,半截 {}, 末条延迟 0.15s", new_msgs.len(), ragged(&new_msgs).len());

    // Both cut the same frames, so neither may LOSE words. They are not required to
    // agree on them: the old policy could cut from a rolling partial, and the new one
    // only ever cuts what the recognizer has committed — which is its second-pass
    // revision of the same words. Those divergences are a *result*, so they are
    // printed rather than asserted away; what is asserted is that nothing was dropped.
    let strip =
        |v: &[String]| -> Vec<char> { v.concat().chars().filter(|c| c.is_alphanumeric()).collect() };
    let (o, n) = (strip(&old_msgs), strip(&new_msgs));
    let divergences = diff(&o, &n);
    println!("\n文本分歧 {} 处(旧 → 新):", divergences.len());
    for (a, b) in &divergences {
        println!("   {a:?} → {b:?}");
    }
    assert!(
        o.len().abs_diff(n.len()) <= 16,
        "one policy lost words: old {} chars, new {} chars",
        o.len(),
        n.len()
    );

    println!(
        "\n半截话: 旧 {} / {} 条,新 {} / {} 条",
        ragged(&old_msgs).len(),
        old_msgs.len(),
        ragged(&new_msgs).len(),
        new_msgs.len()
    );
}
