//! Retained appearance state for `/out/view`, replacing a lossy
//! broadcast.
//!
//! Appearance is *state*, not a stream of utterances: the set of
//! views currently mounted, in z-order. The previous design broadcast each
//! envelope over a `tokio::broadcast`, so a view shown before a client's GET
//! opened — or while a page was refreshing, or before a second device joined —
//! was simply never seen. This bus folds the reaction's show/replace/dismiss
//! envelopes into one retained state and serves the whole of it to any
//! subscriber, so every attached client converges on the same screen no
//! matter when it connects.
//!
//! Sync is a versioned long-poll: `wait_state(since)` returns the full
//! state as soon as the version exceeds `since` (immediately when
//! `since` is absent or behind). State is tiny — a few ids and module URLs —
//! so resending it whole kills the missed-delta bug class outright.
//!
//! A view persists until the agent dismisses or replaces it: there is no
//! auto-expiry, lifetime is the reaction's decision.
//!
//! The state also survives restarts: every mutation appends a whole-state
//! snapshot to the memory store at
//! `raw/appearance/<date>/appearance-<HHMMSSZ>.json`, and
//! [`ViewBus::load`] restores from the newest snapshot on boot. The
//! snapshots double as the appearance history (the screen as
//! expression, for later reflection). Module URLs stay valid across restarts
//! because compiled views are content-addressed on disk and never collected
//! (see [`crate::mind::views`]).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, Notify};

use crate::mind::memory::layout;
use crate::types::{ViewEnvelope, ViewOp, ViewTraits};

/// Retained appearance state. Cloneable handle over shared
/// state.
#[derive(Clone)]
pub struct ViewBus {
    inner: Arc<Mutex<Appearance>>,
    /// The memory data dir; snapshots live under `raw/appearance/`.
    data_dir: PathBuf,
}

/// The screen: two fixed slots, not a stack.
///
/// The screen used to be an open z-ordered list any `show` could push onto, and
/// it accumulated exactly as you'd expect — the appearance history has states
/// fourteen views deep, a dozen unrelated topics piled up because nothing ever
/// dismissed them, all rendered on top of each other in the same centre of the
/// screen. Meanwhile the composition that list was there to allow was never once
/// used deliberately in the entire recorded history.
///
/// So the list is gone and what's left is the one layering that was ever real:
/// the agent's content, and the host's condition notice over it. They are separate
/// slots because they have separate lifetimes and separate owners — an outage
/// arriving must not evict the content it covers, and clearing when the outage
/// lifts must reveal that content again rather than leave a blank screen. Which
/// slot a write lands in is decided by *how* it was written, not by anything on
/// the wire: [`apply`](ViewBus::apply) is the agent's path and owns `content`,
/// [`reconcile`](ViewBus::reconcile) is the process's level-driven path and owns
/// `condition`.
#[derive(Default)]
struct Appearance {
    /// The one agent-shown view, filling the screen. `None` = the empty room.
    content: Option<RetainedView>,
    /// The host's condition layer over the content (e.g. a vendor outage).
    condition: Option<RetainedView>,
    /// Bumped on every state change; the long-poll's `since` compares against it.
    version: u64,
    /// Pulsed whenever `version` bumps so parked readers re-check.
    notify: Arc<Notify>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct RetainedView {
    id: String,
    module_url: String,
    /// What the view declared about itself. `#[serde(default)]` is the back-compat
    /// lever: snapshots written before this field existed reload as `None` →
    /// host-owned captions.
    #[serde(default)]
    traits: Option<ViewTraits>,
}

/// On-disk whole-state snapshot of the appearance at a moment, with `as_of` so
/// the history reads as a step-function of what was on screen.
///
/// `legacy_views` reads the flat z-ordered list snapshots carried before the two
/// slots existed. Those states are exactly the pile-ups the slots abolished, so
/// they restore as the top-most view alone — the one the person was actually
/// looking at — rather than resurrecting a stack that can no longer be
/// represented.
#[derive(Serialize, Deserialize)]
struct Snapshot {
    version: u64,
    as_of: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content: Option<RetainedView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    condition: Option<RetainedView>,
    #[serde(default, rename = "views", skip_serializing_if = "Vec::is_empty")]
    legacy_views: Vec<RetainedView>,
}

/// One active view as delivered to the browser.
#[derive(Debug, Clone, Serialize)]
pub struct WireView {
    pub id: String,
    pub module_url: String,
    /// What the view declared about itself; absent = host-owned captions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traits: Option<ViewTraits>,
}

/// The full appearance state — the body of one `GET /api/out/view`
/// response. `views` is in z-order (first = bottom), so it is at most the
/// content view followed by the host's condition layer. The wire shape is
/// unchanged — an ordered list of full-bleed layers — only what can appear in it
/// is now bounded.
#[derive(Debug, Clone, Serialize)]
pub struct ViewState {
    pub version: u64,
    pub views: Vec<WireView>,
}

impl ViewBus {
    /// Open the bus, restoring the appearance from the newest snapshot under
    /// `raw/appearance/`.
    pub fn load(data_dir: &Path) -> Self {
        let app_dir = layout::raw_root(data_dir).join("appearance");
        let state = match newest_snapshot(&app_dir) {
            // A legacy flat list restores as its top-most view only; see
            // `Snapshot::legacy_views`.
            Some(snap) => Appearance {
                content: snap.content.or_else(|| snap.legacy_views.last().cloned()),
                condition: snap.condition,
                version: snap.version,
                notify: Arc::new(Notify::new()),
            },
            None => Appearance::default(),
        };
        Self {
            inner: Arc::new(Mutex::new(state)),
            data_dir: data_dir.to_path_buf(),
        }
    }

    /// Fold one reaction-emitted envelope into the **content** slot.
    ///
    /// `show` and `replace` both put this view on the screen in place of whatever
    /// was there — the difference is only the id the client keys the slot by, which
    /// is what decides whether a re-render animates or remounts. `dismiss` clears
    /// the slot when it holds the named id.
    ///
    /// Showing is therefore *how the screen changes*, not how it grows: there is no
    /// z-order to raise into and no way for two topics to end up stacked, so the
    /// agent walking someone through a sequence never has to dismiss between beats.
    ///
    /// A write that changes nothing is dropped, so a repeated `show` of what is
    /// already up costs no version bump, no client re-render and no snapshot.
    pub async fn apply(&self, envelope: ViewEnvelope) {
        let mut map = self.inner.lock().await;
        let entry = &mut *map;
        let Some(next) = resolve_slot(&entry.content, envelope) else {
            return;
        };
        if entry.content == next {
            return;
        }
        entry.content = next;
        entry.version += 1;
        entry.notify.notify_waiters();
        persist(&self.data_dir, entry).await;
    }

    /// Reconcile the host-owned **condition** layer against the desired level.
    ///
    /// This is the right write path for process conditions whose current level is
    /// repeatedly re-applied at startup, after lag, and while polling: a dismiss of
    /// an absent condition and a show of one already displayed are both no-ops.
    ///
    /// It writes its own slot, above the content and independent of it. That
    /// separation is the whole reason two slots exist: an outage arriving must not
    /// evict the view it covers, and lifting must reveal that view again rather
    /// than leaving the person on a blank screen.
    pub async fn reconcile(&self, envelope: ViewEnvelope) {
        let mut map = self.inner.lock().await;
        let entry = &mut *map;
        let Some(next) = resolve_slot(&entry.condition, envelope) else {
            return;
        };
        if entry.condition == next {
            return;
        }
        entry.condition = next;
        entry.version += 1;
        entry.notify.notify_waiters();
        persist(&self.data_dir, entry).await;
    }

    /// Whether the retained appearance store holds anything yet. Used by
    /// process-level condition gates to decide whether to reconcile a restored
    /// screen before the HTTP listener starts.
    pub async fn has_state(&self) -> bool {
        let a = self.inner.lock().await;
        a.content.is_some() || a.condition.is_some()
    }

    /// Clear the content — back to the default empty room. A user control:
    /// the screen is the agent's presentation, but the user can reclaim it. Bumps
    /// the version and persists the empty snapshot so every device + a refresh
    /// converge on the cleared screen (and the appearance history records it).
    /// No-op when already empty, so it doesn't churn the version or write a
    /// redundant snapshot.
    ///
    /// The condition layer is deliberately left alone: it reflects a live process
    /// state rather than anything the person put there, so clearing it would only
    /// have it reconciled straight back.
    pub async fn clear(&self) {
        let mut map = self.inner.lock().await;
        let entry = &mut *map;
        if entry.content.is_none() {
            return;
        }
        entry.content = None;
        entry.version += 1;
        entry.notify.notify_waiters();
        persist(&self.data_dir, entry).await;
    }

    /// The id currently on screen, if any. The reaction reads this into
    /// each turn so the agent can *see* its own presentation surface — what it has
    /// shown — instead of guessing ids from the transcript. This is the read side of
    /// the same authoritative state [`apply`](Self::apply) writes, so a view
    /// dismissed last turn is gone from this list the next, giving the agent the
    /// confirmation it otherwise lacks.
    ///
    /// Reports the content slot only. The condition layer is the host's, not the
    /// agent's to dismiss, and listing an id it cannot act on would only invite it
    /// to try.
    pub async fn on_screen(&self) -> Vec<String> {
        self.inner
            .lock()
            .await
            .content
            .as_ref()
            .map(|v| vec![v.id.clone()])
            .unwrap_or_default()
    }

    /// The appearance, as soon as its version exceeds `since`.
    /// `since: None` returns the present state immediately — even when empty —
    /// so a fresh page knows it is synced; passing the last seen version parks
    /// until the state changes.
    pub async fn wait_state(&self, since: Option<u64>) -> ViewState {
        loop {
            let mut map = self.inner.lock().await;
            let entry = &mut *map;
            if since.is_none_or(|s| entry.version > s) {
                return ViewState {
                    version: entry.version,
                    // z-order: content first, the condition layer over it.
                    views: [entry.content.as_ref(), entry.condition.as_ref()]
                        .into_iter()
                        .flatten()
                        .map(|v| WireView {
                            id: v.id.clone(),
                            module_url: v.module_url.clone(),
                            traits: v.traits,
                        })
                        .collect(),
                };
            }
            // Enroll on the notify *while still holding the lock* so a
            // `notify_waiters()` between here and the await cannot be lost,
            // then release the lock and park (same pattern as TextBus).
            let notify = entry.notify.clone();
            let notified = notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            drop(map);
            notified.await;
        }
    }
}

/// What one slot should hold after `envelope` is folded into its `current`
/// occupant. `None` means the envelope was malformed and should be dropped
/// without touching the slot — distinct from `Some(None)`, which empties it.
///
/// The whole fold is this small because a slot holds one view: `show` and
/// `replace` both simply put the new view there, and `dismiss` empties it only
/// when it holds the id being dismissed (so a stale dismiss aimed at something
/// already gone can't blank the screen out from under whatever replaced it).
fn resolve_slot(
    current: &Option<RetainedView>,
    envelope: ViewEnvelope,
) -> Option<Option<RetainedView>> {
    match envelope.op {
        ViewOp::Dismiss => {
            if current.as_ref().is_some_and(|v| v.id == envelope.id) {
                Some(None)
            } else {
                Some(current.clone())
            }
        }
        ViewOp::Show | ViewOp::Replace => {
            let Some(module_url) = envelope.module_url else {
                tracing::warn!(id = %envelope.id, "view envelope without module_url; dropping");
                return None;
            };
            Some(Some(RetainedView {
                id: envelope.id,
                module_url,
                traits: envelope.traits,
            }))
        }
    }
}

/// The newest parseable snapshot under the `appearance/` dir, or `None`.
/// Walks day-folders newest-first, then `appearance-*.json` newest-first, so a
/// torn final write falls back to the prior snapshot.
fn newest_snapshot(appearance_dir: &Path) -> Option<Snapshot> {
    let mut days: Vec<String> = std::fs::read_dir(appearance_dir)
        .ok()?
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    days.sort();
    for day in days.iter().rev() {
        let day_dir = appearance_dir.join(day);
        let mut files: Vec<String> = std::fs::read_dir(&day_dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.starts_with("appearance-") && n.ends_with(".json"))
            .collect();
        files.sort();
        for f in files.iter().rev() {
            if let Ok(bytes) = std::fs::read(day_dir.join(f)) {
                if let Ok(snap) = serde_json::from_slice::<Snapshot>(&bytes) {
                    return Some(snap);
                }
            }
        }
    }
    None
}

/// Append a whole-state snapshot to `raw/appearance/<date>/`. The file
/// is named for the wall-clock second; on the rare same-second collision the
/// second is bumped until free, so no snapshot in the history is overwritten.
/// Tempfile + rename so a crash mid-write never leaves a torn snapshot at a real
/// name. Failures are logged, not fatal — the live state stays authoritative.
async fn persist(data_dir: &Path, entry: &Appearance) {
    let now = Utc::now();
    let snap = Snapshot {
        version: entry.version,
        as_of: now,
        content: entry.content.clone(),
        condition: entry.condition.clone(),
        legacy_views: Vec::new(),
    };
    let bytes = match serde_json::to_vec_pretty(&snap) {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::warn!(error = %err, "encoding appearance snapshot failed");
            return;
        }
    };
    let dir = layout::appearance_day_dir(data_dir, now);
    if let Err(err) = tokio::fs::create_dir_all(&dir).await {
        tracing::warn!(error = %err, "creating appearance dir failed");
        return;
    }
    let mut slot = now;
    let path = loop {
        let p = dir.join(format!("appearance-{}.json", slot.format("%H%M%SZ")));
        if !tokio::fs::try_exists(&p).await.unwrap_or(false) {
            break p;
        }
        slot += Duration::seconds(1);
    };
    let tmp = dir.join(format!(
        ".tmp.{}.{}",
        std::process::id(),
        slot.format("%H%M%S")
    ));
    let result = async {
        tokio::fs::write(&tmp, &bytes).await?;
        tokio::fs::rename(&tmp, &path).await
    }
    .await;
    if let Err(err) = result {
        tracing::warn!(path = %path.display(), error = %err, "persisting appearance snapshot failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn show(id: &str, url: &str) -> ViewEnvelope {
        ViewEnvelope {
            id: id.into(),
            op: ViewOp::Show,
            module_url: Some(url.into()),
            traits: None,
        }
    }

    fn dismiss(id: &str) -> ViewEnvelope {
        ViewEnvelope {
            id: id.into(),
            op: ViewOp::Dismiss,
            module_url: None,
            traits: None,
        }
    }

    fn ids(state: &ViewState) -> Vec<&str> {
        state.views.iter().map(|v| v.id.as_str()).collect()
    }

    #[tokio::test]
    async fn late_subscriber_receives_retained_state() {
        let tmp = tempfile::tempdir().unwrap();
        let bus = ViewBus::load(tmp.path());
        bus.apply(show("a", "/m/a.mjs")).await;

        // No subscriber existed at apply time — the state is still served.
        let state = bus.wait_state(None).await;
        assert_eq!(state.version, 1);
        assert_eq!(ids(&state), vec!["a"]);
        assert_eq!(state.views[0].module_url, "/m/a.mjs");
    }

    #[tokio::test]
    async fn an_empty_screen_returns_immediately() {
        let tmp = tempfile::tempdir().unwrap();
        let bus = ViewBus::load(tmp.path());
        let state = bus.wait_state(None).await;
        assert_eq!(state.version, 0);
        assert!(state.views.is_empty());
    }

    #[tokio::test]
    async fn since_parks_until_next_change() {
        let tmp = tempfile::tempdir().unwrap();
        let bus = ViewBus::load(tmp.path());
        bus.apply(show("a", "/m/a.mjs")).await;
        let v = bus.wait_state(None).await.version;

        let waiter = {
            let bus = bus.clone();
            tokio::spawn(async move { bus.wait_state(Some(v)).await })
        };
        tokio::task::yield_now().await;
        bus.apply(show("b", "/m/b.mjs")).await;

        let state = tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
            .await
            .expect("waiter should wake")
            .unwrap();
        assert_eq!(state.version, v + 1);
        assert_eq!(ids(&state), vec!["b"]);
    }

    /// The core of the redesign: the screen holds one agent view, so showing a
    /// second topic *replaces* the first instead of stacking on it. This is the
    /// case that used to pile up — the appearance history has states fourteen
    /// views deep, every one of them an unrelated topic nobody dismissed.
    #[tokio::test]
    async fn showing_replaces_rather_than_stacks() {
        let tmp = tempfile::tempdir().unwrap();
        let bus = ViewBus::load(tmp.path());
        bus.apply(show("tasks", "/m/tasks.mjs")).await;
        bus.apply(show("bj01", "/m/bj01.mjs")).await;

        let state = bus.wait_state(None).await;
        assert_eq!(
            ids(&state),
            vec!["bj01"],
            "the older topic is gone, not stacked under"
        );
        assert_eq!(state.views.len(), 1);
    }

    #[tokio::test]
    async fn replace_swaps_the_module_under_a_kept_id() {
        let tmp = tempfile::tempdir().unwrap();
        let bus = ViewBus::load(tmp.path());
        bus.apply(show("deck", "/m/v1.mjs")).await;
        bus.apply(
            ViewEnvelope {
                id: "deck".into(),
                op: ViewOp::Replace,
                module_url: Some("/m/v2.mjs".into()),
                traits: None,
            },
        )
        .await;

        let state = bus.wait_state(None).await;
        assert_eq!(ids(&state), vec!["deck"]);
        assert_eq!(state.views[0].module_url, "/m/v2.mjs");
    }

    #[tokio::test]
    async fn dismiss_clears_only_the_id_it_names() {
        let tmp = tempfile::tempdir().unwrap();
        let bus = ViewBus::load(tmp.path());
        bus.apply(show("current", "/m/c.mjs")).await;

        // A stale dismiss aimed at something already replaced must not blank the
        // screen out from under whatever took its place.
        bus.apply(dismiss("long-gone")).await;
        assert_eq!(ids(&bus.wait_state(None).await), vec!["current"]);

        bus.apply(dismiss("current")).await;
        assert!(bus.wait_state(None).await.views.is_empty());
    }

    #[tokio::test]
    async fn repeated_identical_show_is_a_no_op() {
        let tmp = tempfile::tempdir().unwrap();
        let bus = ViewBus::load(tmp.path());
        bus.apply(show("a", "/m/a.mjs")).await;
        let v = bus.wait_state(None).await.version;
        bus.apply(show("a", "/m/a.mjs")).await;
        assert_eq!(
            bus.wait_state(None).await.version,
            v,
            "re-showing what is already up must not churn the version"
        );
    }

    #[tokio::test]
    async fn clear_empties_and_wakes() {
        let tmp = tempfile::tempdir().unwrap();
        let bus = ViewBus::load(tmp.path());

        // Clearing an already-empty screen is a no-op: version stays at 0.
        bus.clear().await;
        assert_eq!(bus.wait_state(None).await.version, 0);

        bus.apply(show("a", "/m/a.mjs")).await;
        let v = bus.wait_state(None).await.version;

        // A parked reader wakes on the clear with the empty, version-bumped state.
        let waiter = {
            let bus = bus.clone();
            tokio::spawn(async move { bus.wait_state(Some(v)).await })
        };
        tokio::task::yield_now().await;
        bus.clear().await;

        let state = tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
            .await
            .expect("waiter should wake")
            .unwrap();
        assert_eq!(state.version, v + 1);
        assert!(state.views.is_empty());
    }

    /// The one layering that was ever real, and the reason there are two slots
    /// rather than one: an outage lands *over* the content without evicting it,
    /// and lifting reveals what was underneath instead of a blank screen.
    #[tokio::test]
    async fn condition_layers_over_content_and_restores_it() {
        let tmp = tempfile::tempdir().unwrap();
        let bus = ViewBus::load(tmp.path());
        bus.apply(show("feishu-board", "/m/board.mjs")).await;
        bus.reconcile(show("vendor-outage", "/m/energy.mjs"))
            .await;

        let state = bus.wait_state(None).await;
        assert_eq!(
            ids(&state),
            vec!["feishu-board", "vendor-outage"],
            "content first, the condition layer over it"
        );

        bus.reconcile(dismiss("vendor-outage")).await;
        assert_eq!(
            ids(&bus.wait_state(None).await),
            vec!["feishu-board"],
            "lifting the outage reveals the content it covered"
        );
    }

    /// The two slots are independent: the agent showing a new topic must not
    /// disturb a live outage, and clearing the screen must not either.
    #[tokio::test]
    async fn content_writes_leave_the_condition_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let bus = ViewBus::load(tmp.path());
        bus.reconcile(show("vendor-outage", "/m/energy.mjs"))
            .await;

        bus.apply(show("a", "/m/a.mjs")).await;
        bus.apply(show("b", "/m/b.mjs")).await;
        assert_eq!(
            ids(&bus.wait_state(None).await),
            vec!["b", "vendor-outage"]
        );

        bus.apply(dismiss("b")).await;
        assert_eq!(ids(&bus.wait_state(None).await), vec!["vendor-outage"]);

        bus.apply(show("c", "/m/c.mjs")).await;
        bus.clear().await;
        assert_eq!(
            ids(&bus.wait_state(None).await),
            vec!["vendor-outage"],
            "a user clear reclaims the content, not the live condition"
        );
    }

    /// An agent `dismiss` naming the condition's id must not reach it — the two
    /// slots are addressed by write path, not by id.
    #[tokio::test]
    async fn the_agent_cannot_dismiss_the_condition_layer() {
        let tmp = tempfile::tempdir().unwrap();
        let bus = ViewBus::load(tmp.path());
        bus.reconcile(show("vendor-outage", "/m/energy.mjs"))
            .await;
        bus.apply(dismiss("vendor-outage")).await;
        assert_eq!(ids(&bus.wait_state(None).await), vec!["vendor-outage"]);
    }

    #[tokio::test]
    async fn reconcile_is_level_driven_and_does_not_churn_versions() {
        let tmp = tempfile::tempdir().unwrap();
        let bus = ViewBus::load(tmp.path());
        let energy = show("vendor-outage", "/m/energy.mjs");

        bus.reconcile(energy.clone()).await;
        let shown = bus.wait_state(None).await;
        assert_eq!(shown.version, 1);
        assert_eq!(ids(&shown), vec!["vendor-outage"]);

        bus.reconcile(energy).await;
        assert_eq!(
            bus.wait_state(None).await.version,
            shown.version,
            "re-applying the same condition must be a no-op"
        );

        bus.reconcile(dismiss("vendor-outage")).await;
        let hidden = bus.wait_state(None).await;
        assert_eq!(hidden.version, shown.version + 1);
        assert!(hidden.views.is_empty());

        bus.reconcile(dismiss("vendor-outage")).await;
        assert_eq!(
            bus.wait_state(None).await.version,
            hidden.version,
            "dismissing an absent condition must be a no-op"
        );
    }

    #[tokio::test]
    async fn persists_and_reloads_across_restart() {
        let tmp = tempfile::tempdir().unwrap();
        let version = {
            let bus = ViewBus::load(tmp.path());
            bus.apply(show("a", "/m/a.mjs")).await;
            bus.apply(show("b", "/m/b.mjs")).await;
            bus.reconcile(show("vendor-outage", "/m/energy.mjs"))
                .await;
            bus.wait_state(None).await.version
        };

        // "Restart": a fresh bus over the same data dir restores the newest
        // snapshot, both slots intact.
        let bus = ViewBus::load(tmp.path());
        let state = bus.wait_state(None).await;
        assert_eq!(state.version, version);
        assert_eq!(ids(&state), vec!["b", "vendor-outage"]);
        assert_eq!(state.views[0].module_url, "/m/b.mjs");
    }

    /// The screen survives a restart: a view shown before the process went down
    /// is on screen again when it comes back, at the same version.
    #[tokio::test]
    async fn reload_restores_the_screen() {
        let tmp = tempfile::tempdir().unwrap();
        let version = {
            let bus = ViewBus::load(tmp.path());
            bus.apply(show("keep", "/m/k.mjs")).await;
            bus.wait_state(None).await.version
        };

        let bus = ViewBus::load(tmp.path());
        let state = bus.wait_state(None).await;
        assert_eq!(state.version, version);
        assert_eq!(ids(&state), vec!["keep"]);
    }

    #[tokio::test]
    async fn loads_legacy_stacked_snapshot_as_its_topmost_view() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = layout::appearance_day_dir(tmp.path(), Utc::now());
        std::fs::create_dir_all(&dir).unwrap();
        let old = r#"{"version":3423,"as_of":"2026-08-07T09:05:30Z","views":[
            {"id":"tasks","module_url":"/m/tasks.mjs","geometry":{"region":"center","size":"wide"}},
            {"id":"bj01","module_url":"/m/bj01.mjs","geometry":{"region":"center","size":"auto"}}
        ]}"#;
        std::fs::write(dir.join("appearance-090530Z.json"), old).unwrap();

        let bus = ViewBus::load(tmp.path());
        let state = bus.wait_state(None).await;
        assert_eq!(state.version, 3423);
        assert_eq!(ids(&state), vec!["bj01"]);
        assert!(
            state.views[0].traits.is_none(),
            "retired geometry keys are ignored"
        );
    }

    #[tokio::test]
    async fn apply_carries_traits_through_wire_and_reload() {
        let tmp = tempfile::tempdir().unwrap();
        let traits = ViewTraits {
            owns_captions: true,
        };

        let version = {
            let bus = ViewBus::load(tmp.path());
            bus.apply(
                ViewEnvelope {
                    id: "g".into(),
                    op: ViewOp::Show,
                    module_url: Some("/m/g.mjs".into()),
                    traits: Some(traits),
                },
            )
            .await;
            let state = bus.wait_state(None).await;
            assert_eq!(state.views[0].traits, Some(traits));
            state.version
        };

        // Traits ride the snapshot, so they survive a restart.
        let bus = ViewBus::load(tmp.path());
        let state = bus.wait_state(None).await;
        assert_eq!(state.version, version);
        assert_eq!(state.views[0].traits, Some(traits));
    }

    #[tokio::test]
    async fn on_screen_reports_the_content_slot_only() {
        let tmp = tempfile::tempdir().unwrap();
        let bus = ViewBus::load(tmp.path());
        assert!(bus.on_screen().await.is_empty());

        bus.apply(show("a", "/m/a.mjs")).await;
        bus.reconcile(show("vendor-outage", "/m/energy.mjs"))
            .await;
        assert_eq!(
            bus.on_screen().await,
            vec!["a".to_string()],
            "the agent is told what it can act on, not the host's layer"
        );
    }
}
