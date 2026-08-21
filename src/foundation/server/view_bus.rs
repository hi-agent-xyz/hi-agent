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
//!
//! The state also carries `history`: the recent shows, oldest first, so a person
//! can go back to something the agent has moved past. It is the *record of shows*,
//! not of anybody's navigation — a window's position in it is that window's own and
//! is never reported here, exactly as scroll position is never reported for the
//! conversation. There is one list and one cursor per window, and because a show
//! only ever appends, no navigation can be truncated by one arriving.

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
    /// Where the person last went, if that is not what the agent has up.
    ///
    /// **Deliberately outside [`Appearance`].** Everything in there is the appearance:
    /// it is versioned, it goes out on `GET /api/out/view`, and it is snapshotted. This
    /// is none of those things — it is a perception the turn reads, and the moment it
    /// lived beside the slots someone would serialize it and a phone would start moving
    /// the desktop. Its own lock is the cheapest way to make that impossible rather
    /// than merely discouraged. See
    /// `docs/arch/stage.md#where-they-went-is-reported-the-cursor-still-is-not`.
    attention: Arc<Mutex<Option<Attention>>>,
    /// The memory data dir; snapshots live under `raw/appearance/`.
    data_dir: PathBuf,
}

/// Where the person went, and when.
///
/// One value for the whole install, not one per window: one person owns an install
/// (`docs/arch/topology.md`), so two windows are two of their eyes, and the newest move
/// is the best available answer to where they are looking.
#[derive(Debug, Clone, PartialEq)]
pub struct Attention {
    /// What identifies the *place*, by the same rule the history dedupes by: the
    /// durable ref when there is one, else the compiled module. Never rendered — it
    /// exists so a show onto the same place can recognize itself and clear this.
    pub key: String,
    /// What to call the destination in a prompt — the ref when it has one, else the
    /// id the inline view was shown as. The module hash names nothing.
    pub name: String,
    /// When they went there. Rendered as an age, never as a claim about now: a window
    /// that reloaded is live again and never says so, so this fact can outlive the
    /// looking and the turn has to be able to weigh it.
    pub at: DateTime<Utc>,
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
    /// What has been shown into `content`, oldest first — the newest entry is
    /// what is up now. See [`record_show`].
    history: Vec<HistoryEntry>,
    /// Bumped on every state change; the long-poll's `since` compares against it.
    version: u64,
    /// Pulsed whenever `version` bumps so parked readers re-check.
    notify: Arc<Notify>,
}

/// How many shows the history keeps. Bounded because the point of it is reaching
/// something the agent has moved past within a working stretch, not archiving the
/// day — the snapshots under `raw/appearance/` are the archive, and a fuller browser
/// over them can be its own view. Bounded also keeps the state small enough that
/// resending it whole on every version bump stays the right design.
const HISTORY_MAX: usize = 24;

/// One show, and when it happened.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct HistoryEntry {
    view: RetainedView,
    at: DateTime<Utc>,
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
    /// The view's durable name — see [`ViewEnvelope::view_ref`]. A snapshot records
    /// it so [`ViewBus::refresh_sources`] can recompile the view on the next boot
    /// instead of resurrecting the module it happened to compile to. `None` for an
    /// inline-source view, and for every snapshot written before this field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    view_ref: Option<String>,
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
    /// The recent shows, oldest first. `#[serde(default)]` is the back-compat
    /// lever: a snapshot written before the history existed reloads with an empty
    /// one, which is exactly right — nothing is known to have been shown, so there
    /// is nowhere to go back to until the agent shows something.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    history: Vec<HistoryEntry>,
    #[serde(default, rename = "views", skip_serializing_if = "Vec::is_empty")]
    legacy_views: Vec<RetainedView>,
}

/// Which slot a delivered layer came out of.
///
/// The wire used to carry only z-order, which is enough to paint and not enough to
/// compose: a window showing a *past* view in place of the live one has to keep the
/// condition layer over it, and could not tell which of the two layers that was. An
/// outage must still cover what the person went back to.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WireSlot {
    Content,
    Condition,
}

/// One active view as delivered to the browser.
#[derive(Debug, Clone, Serialize)]
pub struct WireView {
    pub id: String,
    pub module_url: String,
    /// What the view declared about itself; absent = host-owned captions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traits: Option<ViewTraits>,
    pub slot: WireSlot,
}

/// One past show as delivered to the browser.
///
/// `label` is derived here rather than declared by the view, because a view declares
/// nothing but `owns_conversation` and inventing a title field would make every
/// existing view untitled. The ref's last segment is the honest name — it is what the
/// agent typed to show it and what the file is called — and an inline view falls back
/// to its id, the way a browser falls back to a URL for a page with no `<title>`.
///
/// `view_ref` is the lever that decides what re-opening means: a named view is
/// re-resolved from its current source (so `factory/tasks` reopens as today's board),
/// while an inline view can only ever come back as the artifact it compiled to. The
/// client sends the ref back when there is one, and mounts `module_url` when there
/// isn't. This is the same named/inline split [`ViewBus::refresh_sources`] turns on.
#[derive(Debug, Clone, Serialize)]
pub struct WireHistoryEntry {
    pub id: String,
    pub module_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view_ref: Option<String>,
    pub label: String,
    pub at: DateTime<Utc>,
    /// A picture of this show, served from `/views/_shots/<hash>.png` — absent while
    /// the capture is still running, and for good on a view that did not render
    /// cleanly. The tile falls back to its mark either way, so this is decoration on
    /// a record that is complete without it. See [`super::view_shots`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shot_url: Option<String>,
}

/// The full appearance state — the body of one `GET /api/out/view`
/// response. `views` is in z-order (first = bottom), so it is at most the
/// content view followed by the host's condition layer. The wire shape is
/// unchanged — an ordered list of full-bleed layers — only what can appear in it
/// is now bounded.
///
/// `history` rides in the same response rather than getting its own endpoint,
/// because it changes exactly when the appearance does: one long-poll, one version,
/// and no second sync path that could disagree with the first about what is up.
#[derive(Debug, Clone, Serialize)]
pub struct ViewState {
    pub version: u64,
    pub views: Vec<WireView>,
    /// The recent shows, oldest first. The last entry is what is on the stage now,
    /// which is what makes "the person is at the end" mean "the person is live".
    pub history: Vec<WireHistoryEntry>,
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
                history: snap.history,
                version: snap.version,
                notify: Arc::new(Notify::new()),
            },
            None => Appearance::default(),
        };
        Self {
            inner: Arc::new(Mutex::new(state)),
            // In-memory only, and not restored: after a restart the agent has no idea
            // where anybody is looking, which is the truth.
            attention: Arc::new(Mutex::new(None)),
            data_dir: data_dir.to_path_buf(),
        }
    }

    /// The person went to `name` — the inbound half of the view channel.
    ///
    /// Returns whether this actually moved them, so the caller can journal a move and
    /// stay quiet about a re-click of the tile they are already on.
    ///
    /// `live` says the destination is what the agent currently has up, which is not a
    /// place to be told about: it clears the fact instead of recording one, so the
    /// turn's context says nothing rather than saying something it would have to
    /// qualify away.
    pub async fn note_went_to(&self, key: String, name: String, live: bool) -> bool {
        let mut slot = self.attention.lock().await;
        if live {
            return slot.take().is_some();
        }
        if slot.as_ref().is_some_and(|a| a.key == key) {
            return false;
        }
        *slot = Some(Attention { key, name, at: Utc::now() });
        true
    }

    /// Forget where they went, because the agent just shown something.
    ///
    /// This is the client's own rule for being live again — a show takes the window with
    /// it, whatever it had gone back to (`docs/arch/stage.md`) — made here, in the same
    /// call that records the show, so the two cannot drift. It used to clear only when the
    /// show landed on the very destination they had gone to, which was right while a
    /// parked window stayed put and is wrong now that none does: the alternative is telling
    /// the agent they are away reading something it has just taken them off.
    async fn caught_up_by_show(&self) {
        *self.attention.lock().await = None;
    }

    /// Where the person went, if they went anywhere and have not been caught up with.
    /// Read into each turn beside [`on_screen`](Self::on_screen).
    pub async fn attention(&self) -> Option<Attention> {
        self.attention.lock().await.clone()
    }

    /// Fold one reaction-emitted envelope into the **content** slot.
    ///
    /// `show` and `replace` both put this view on the screen in place of whatever
    /// was there — the difference is only the id the client keys the slot by, which
    /// is what decides whether a re-render animates or remounts. `dismiss` clears
    /// the slot when it holds the named id.
    ///
    /// Showing is therefore *how the screen changes*, not how it grows: there is no
    /// z-order left to push onto and no way for two topics to end up stacked, so the
    /// agent walking someone through a sequence never has to dismiss between beats.
    ///
    /// A write that changes nothing is dropped, so a repeated `show` of what is
    /// already up costs no version bump, no client re-render and no snapshot.
    ///
    /// A show is also recorded in `history`, so the person can reach it again after
    /// the agent has moved on. A dismiss is not: the empty room is not somewhere to
    /// go back to, and the view it cleared is already in the list.
    pub async fn apply(&self, envelope: ViewEnvelope) {
        let mut map = self.inner.lock().await;
        let entry = &mut *map;
        let Some(next) = resolve_slot(&entry.content, envelope) else {
            return;
        };
        if entry.content == next {
            return;
        }
        if let Some(shown) = &next {
            record_show(&mut entry.history, shown.clone(), Utc::now());
            self.caught_up_by_show().await;
            // Take the picture now, while this *is* the screen. Off the write path
            // entirely: nothing here waits for a browser, and a shot that never
            // arrives leaves the tile on its mark.
            let bus = self.clone();
            let done = move || {
                tokio::spawn(async move { bus.note_shot().await });
            };
            // A named view is a standing surface — its picture is filed under the ref
            // and re-taken as the board moves; an inline view is only ever the artifact
            // it compiled to, and its picture is written once. Same split the whole
            // history turns on. See [`super::view_shots`].
            match shown.view_ref.clone() {
                Some(view_ref) => super::view_shots::capture_ref(
                    self.data_dir.clone(),
                    view_ref,
                    shown.module_url.clone(),
                    done,
                ),
                None => super::view_shots::capture(
                    self.data_dir.clone(),
                    shown.module_url.clone(),
                    done,
                ),
            }
        }
        entry.content = next;
        entry.version += 1;
        entry.notify.notify_waiters();
        persist(&self.data_dir, entry).await;
    }

    /// Recompile the restored screen from its source, so a restart shows what the
    /// view *is* now rather than the module it happened to compile to when it was
    /// shown.
    ///
    /// [`ViewBus::load`] restores `module_url` verbatim, and that URL is a content
    /// hash over the source **as it was at show time**; the compiled tree is a
    /// disposable cache, not the view's identity. Without this pass, editing a view's
    /// source — or shipping a binary that reseeds `factory/` — leaves the old
    /// artifact pinned on screen forever. Nothing errors, because the old module is
    /// still on disk and still imports cleanly; the screen just quietly keeps serving
    /// a version of the view the rest of the process has moved past. That is not
    /// hypothetical: a `factory/tasks` compiled before the task lifecycle replaced
    /// `state` with `status` went on filtering `x.state === "open"` against an API
    /// that had stopped emitting `state`, and so reported every task closed and the
    /// list empty.
    ///
    /// Runs once at startup — after `install_factory_views` has reseeded the tree and
    /// after the compiler exists. Only the content slot is refreshed; the condition
    /// slot is re-derived from embedded source by its own reconcile.
    ///
    /// Failure is deliberately not fatal, and never blanks the screen: a ref that no
    /// longer resolves or source that no longer compiles keeps the pinned module. A
    /// stale view beats an empty room, and the person can always clear it.
    pub async fn refresh_sources(&self, compiler: &crate::mind::views::ViewCompiler) {
        let restored = { self.inner.lock().await.content.clone() };
        let Some(view) = restored else {
            return;
        };
        let Some(view_ref) = restored_ref(&view, &self.data_dir).await else {
            return;
        };

        let (source, traits) = match crate::mind::views::resolve_ref(&self.data_dir, &view_ref).await
        {
            Ok(resolved) => resolved,
            Err(error) => {
                tracing::warn!(
                    id = %view.id, view_ref = %view_ref, %error,
                    "restored view's source is gone; keeping the module it was shown as",
                );
                return;
            }
        };
        let module_url = match compiler.compile(&source).await {
            Ok(url) => url,
            Err(error) => {
                tracing::warn!(
                    id = %view.id, view_ref = %view_ref, %error,
                    "restored view no longer compiles; keeping the module it was shown as",
                );
                return;
            }
        };

        let next = RetainedView {
            id: view.id.clone(),
            module_url,
            traits,
            view_ref: Some(view_ref),
        };

        let mut map = self.inner.lock().await;
        let entry = &mut *map;
        // Anything that wrote the slot while we were compiling is newer than what we
        // restored, so it wins — we would otherwise put the old screen back.
        if entry.content.as_ref() != Some(&view) || entry.content.as_ref() == Some(&next) {
            return;
        }
        tracing::info!(
            id = %next.id,
            view_ref = next.view_ref.as_deref().unwrap_or(""),
            was = %view.module_url,
            now = %next.module_url,
            "recompiled the restored view from source",
        );
        entry.content = Some(next);
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
    ///
    /// The history is left alone too. Reclaiming the screen says "not now", not
    /// "forget what you showed me" — and a clear that also wiped the way back to the
    /// view being cleared would make the reclaim the most destructive control in the
    /// product.
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

    /// A thumbnail finished rendering: wake the long-polls so they collect the
    /// `shot_url` the response before them was built without.
    ///
    /// **Bumps the version without persisting a snapshot.** The snapshots under
    /// `raw/appearance/` are the record of what was on screen, and a picture taken of
    /// a show that already happened changes nothing about that — writing one would
    /// put a state in the appearance history identical to its predecessor and dated
    /// later, which is exactly the noise reflection has to read past. The version is
    /// only the long-poll's comparator, so it is free to run ahead of the newest
    /// snapshot; a restart resyncs every client from `since: None` regardless.
    pub(super) async fn note_shot(&self) {
        let mut map = self.inner.lock().await;
        map.version += 1;
        map.notify.notify_waiters();
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
                    views: [
                        (WireSlot::Content, entry.content.as_ref()),
                        (WireSlot::Condition, entry.condition.as_ref()),
                    ]
                    .into_iter()
                    .filter_map(|(slot, v)| v.map(|v| (slot, v)))
                    .map(|(slot, v)| WireView {
                        id: v.id.clone(),
                        module_url: v.module_url.clone(),
                        traits: v.traits,
                        slot,
                    })
                    .collect(),
                    history: entry
                        .history
                        .iter()
                        .map(|h| WireHistoryEntry {
                            id: h.view.id.clone(),
                            module_url: h.view.module_url.clone(),
                            view_ref: h.view.view_ref.clone(),
                            label: label_for(&h.view),
                            at: h.at,
                            shot_url: shot_for(&self.data_dir, &h.view),
                        })
                        .collect(),
                };
            }
            // Enroll on the notify *while still holding the lock* so a
            // `notify_waiters()` between here and the await cannot be lost,
            // then release the lock and park.
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
                view_ref: envelope.view_ref,
            }))
        }
    }
}

/// Record one show at the end of `history`, dropping any earlier entry for the same
/// destination and trimming the oldest away past [`HISTORY_MAX`].
///
/// **Appending is the only thing that ever happens to this list**, and that is what
/// makes it safe for the agent and the person to share one of them. A browser's back
/// stack destroys its forward entries when you navigate from a back position, and it
/// can afford to because you are its only navigator; here the agent shows views too,
/// and losing the entry a person was on their way back to because the agent spoke
/// would be indefensible. So a show never truncates, and a person's position is just
/// a cursor over the list — there is no branch to destroy.
///
/// **Same destination, one entry.** A destination is the `view_ref` when there is one
/// and the `module_url` when there isn't, which is precisely the identity that decides
/// what re-opening will render: two shows of `factory/tasks` resolve to the same
/// recompiled board, so two tiles would offer one place twice. Two *different* inline
/// views have different content hashes and both stay. The surviving entry moves to the
/// end and takes the newer timestamp, because what matters about it is when the screen
/// last showed it.
/// The picture to hang on one past show.
///
/// A named view's tile is its **surface** picture, not a picture of the moment it went
/// up: re-opening `factory/tasks` deliberately re-resolves to today's board, so a tile
/// promising last Tuesday's would be a wrong picture of the place the card leads to.
/// An inline view has no surface to be current about and keeps its artifact's shot —
/// and so does a named entry whose surface picture has not been taken yet, which is
/// how every show recorded before this split keeps the picture it already had.
fn shot_for(data_dir: &Path, view: &RetainedView) -> Option<String> {
    view.view_ref
        .as_deref()
        .and_then(|r| super::view_shots::url_for_ref(data_dir, r))
        .or_else(|| super::view_shots::url_for(data_dir, &view.module_url))
}

fn record_show(history: &mut Vec<HistoryEntry>, view: RetainedView, at: DateTime<Utc>) {
    let key = destination_of(&view);
    history.retain(|h| destination_of(&h.view) != key);
    history.push(HistoryEntry { view, at });
    let overflow = history.len().saturating_sub(HISTORY_MAX);
    history.drain(..overflow);
}

/// What identifies a *destination*: the durable ref when there is one, else the compiled
/// module. Two shows of `factory/tasks` are one place because both re-resolve to the same
/// recompiled board; two different inline views are two artifacts and both stay. The client
/// keys its cursor by the same rule (`destinationOf` in `core/views.tsx`) and the inbound
/// half of the view channel reports it, so all three agree on what "the same place" means.
fn destination_of(view: &RetainedView) -> String {
    view.view_ref.clone().unwrap_or_else(|| view.module_url.clone())
}

/// A human label for one past show.
///
/// The ref's last segment with its separators opened up and its first letter shown:
/// `factory/people-review` → `People review`. An inline view has no ref and falls back
/// to its id. Nothing here reads the view's source — the label is wanted for every
/// entry in the state on every version bump, and a dozen file reads per long-poll
/// response to recover a nicer string is the wrong trade. (The `// purpose:` line the
/// factory views open with is the nicer string, if this ever proves too thin.)
fn label_for(view: &RetainedView) -> String {
    humanize_ref(view.view_ref.as_deref().unwrap_or(&view.id))
}

/// `factory/people-review` → `People review`. Shared with the view inventory
/// (`GET /api/views`), so a view carries the same name wherever the person meets it.
pub(crate) fn humanize_ref(view_ref: &str) -> String {
    let last = view_ref.rsplit('/').next().unwrap_or(view_ref);
    let opened = last.replace(['-', '_'], " ");
    let mut chars = opened.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => view_ref.to_string(),
    }
}

/// The ref to recompile a restored view from, or `None` to keep it as it is.
///
/// Normally the snapshot recorded one. Snapshots written before `view_ref` existed
/// did not — and the views most hurt by that are exactly the built-ins, because they
/// are the ones a binary update reseeds, so their pinned module goes stale with
/// nobody having touched a file.
///
/// For those, the id is a sound bridge: `reaction.md` names the built-ins to the
/// agent as `factory/<name>` and it shows each one under that bare name, so an id
/// that matches a file the host itself just seeded into `factory/` names that view.
/// The file is checked rather than assumed, so an id that means nothing there is
/// simply left alone, and the refreshed snapshot records a real ref — the guess
/// happens once per install and never again.
///
/// An inline-source view legitimately has no ref and never gains one; it can only
/// ever be restored as the artifact it compiled to.
async fn restored_ref(view: &RetainedView, data_dir: &Path) -> Option<String> {
    if let Some(view_ref) = &view.view_ref {
        return Some(view_ref.clone());
    }
    let candidate = format!("factory/{}", view.id.trim());
    if !crate::mind::views::valid_ref(&candidate) {
        return None;
    }
    let source = data_dir.join("views").join(format!("{candidate}.jsx"));
    tokio::fs::try_exists(&source).await.unwrap_or(false).then_some(candidate)
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
        history: entry.history.clone(),
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
            view_ref: None,
        }
    }

    fn dismiss(id: &str) -> ViewEnvelope {
        ViewEnvelope {
            id: id.into(),
            op: ViewOp::Dismiss,
            module_url: None,
            traits: None,
            view_ref: None,
        }
    }

    fn ids(state: &ViewState) -> Vec<&str> {
        state.views.iter().map(|v| v.id.as_str()).collect()
    }

    fn history_ids(state: &ViewState) -> Vec<&str> {
        state.history.iter().map(|h| h.id.as_str()).collect()
    }

    /// The person going somewhere is a perception and nothing else: it must not reach
    /// the appearance. If this ever fails, a phone that went back is moving the desktop.
    #[tokio::test]
    async fn a_move_changes_nothing_about_the_appearance() {
        let tmp = tempfile::tempdir().unwrap();
        let bus = ViewBus::load(tmp.path());
        bus.apply(show("a", "/m/a.mjs")).await;
        let before = bus.wait_state(None).await;

        assert!(bus.note_went_to("factory/drive".into(), "factory/drive".into(), false).await);

        let after = bus.wait_state(None).await;
        assert_eq!(after.version, before.version, "a move is not a state change");
        assert_eq!(ids(&after), ids(&before));
        assert_eq!(history_ids(&after), history_ids(&before));
    }

    /// The log records moves, so a re-click of the tile they are already on is not one.
    #[tokio::test]
    async fn going_where_they_already_are_is_not_a_move() {
        let tmp = tempfile::tempdir().unwrap();
        let bus = ViewBus::load(tmp.path());

        assert!(bus.note_went_to("factory/drive".into(), "factory/drive".into(), false).await);
        assert!(!bus.note_went_to("factory/drive".into(), "factory/drive".into(), false).await);
        assert!(bus.note_went_to("factory/tasks".into(), "factory/tasks".into(), false).await);
        assert_eq!(bus.attention().await.unwrap().name, "factory/tasks");
    }

    /// Back on what the agent has up is not somewhere to tell it about.
    #[tokio::test]
    async fn coming_back_to_live_clears_the_fact() {
        let tmp = tempfile::tempdir().unwrap();
        let bus = ViewBus::load(tmp.path());

        bus.note_went_to("factory/drive".into(), "factory/drive".into(), false).await;
        assert!(bus.note_went_to(String::new(), String::new(), true).await);
        assert!(bus.attention().await.is_none());
        // And coming back twice is one move, like any other repeat.
        assert!(!bus.note_went_to(String::new(), String::new(), true).await);
    }

    /// Any show catches up with them, not only one onto the place they went. The window
    /// follows a show, so a fact saying they are elsewhere is not stale — it is false, and
    /// the agent would answer the wrong board confidently on the strength of it.
    #[tokio::test]
    async fn any_show_catches_up_with_them() {
        let tmp = tempfile::tempdir().unwrap();
        let bus = ViewBus::load(tmp.path());
        bus.note_went_to("/m/drive.mjs".into(), "drive".into(), false).await;

        // Somewhere else entirely: they are taken there, so the fact goes.
        bus.apply(show("tasks", "/m/tasks.mjs")).await;
        assert!(bus.attention().await.is_none());
    }

    /// A dismiss is not a show: it clears the slot rather than putting a window on
    /// something, so there is nowhere it could have taken them and the fact stands.
    #[tokio::test]
    async fn a_dismiss_leaves_where_they_went_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let bus = ViewBus::load(tmp.path());
        bus.apply(show("tasks", "/m/tasks.mjs")).await;
        bus.note_went_to("/m/drive.mjs".into(), "drive".into(), false).await;

        bus.apply(dismiss("tasks")).await;
        assert!(bus.attention().await.is_some());
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
    async fn a_show_the_screen_moved_past_stays_reachable_in_history() {
        let tmp = tempfile::tempdir().unwrap();
        let bus = ViewBus::load(tmp.path());
        bus.apply(show("tasks", "/m/tasks.mjs")).await;
        bus.apply(show("bj01", "/m/bj01.mjs")).await;

        let state = bus.wait_state(None).await;
        assert_eq!(ids(&state), vec!["bj01"], "one slot, as before");
        assert_eq!(
            history_ids(&state),
            vec!["tasks", "bj01"],
            "oldest first, and the newest entry is what is up now"
        );
    }

    #[tokio::test]
    async fn dismissing_leaves_the_way_back_to_what_was_dismissed() {
        let tmp = tempfile::tempdir().unwrap();
        let bus = ViewBus::load(tmp.path());
        bus.apply(show("tasks", "/m/tasks.mjs")).await;
        bus.apply(dismiss("tasks")).await;

        let state = bus.wait_state(None).await;
        assert!(state.views.is_empty(), "the room is empty");
        assert_eq!(
            history_ids(&state),
            vec!["tasks"],
            "the empty room is not a place to go back to, but the view it cleared is"
        );
    }

    #[tokio::test]
    async fn clearing_the_screen_does_not_clear_the_way_back() {
        let tmp = tempfile::tempdir().unwrap();
        let bus = ViewBus::load(tmp.path());
        bus.apply(show("tasks", "/m/tasks.mjs")).await;
        bus.clear().await;

        let state = bus.wait_state(None).await;
        assert!(state.views.is_empty());
        assert_eq!(history_ids(&state), vec!["tasks"], "reclaiming says not now, not forget");
    }

    #[tokio::test]
    async fn the_same_named_view_shown_twice_is_one_entry_at_the_newer_position() {
        let tmp = tempfile::tempdir().unwrap();
        let bus = ViewBus::load(tmp.path());
        bus.apply(show_ref("tasks", "/m/tasks.mjs", "factory/tasks")).await;
        bus.apply(show_ref("drive", "/m/drive.mjs", "factory/drive")).await;
        // Same ref, recompiled to a different module — still the same destination.
        bus.apply(show_ref("tasks", "/m/tasks-v2.mjs", "factory/tasks")).await;

        let state = bus.wait_state(None).await;
        assert_eq!(
            history_ids(&state),
            vec!["drive", "tasks"],
            "one tile per destination, moved to the newest position"
        );
        assert_eq!(state.history[1].module_url, "/m/tasks-v2.mjs");
    }

    #[tokio::test]
    async fn two_different_inline_views_both_stay() {
        let tmp = tempfile::tempdir().unwrap();
        let bus = ViewBus::load(tmp.path());
        bus.apply(show("a", "/m/aaa.mjs")).await;
        bus.apply(show("b", "/m/bbb.mjs")).await;

        let state = bus.wait_state(None).await;
        assert_eq!(
            history_ids(&state),
            vec!["a", "b"],
            "distinct artifacts are distinct destinations"
        );
        assert!(
            state.history.iter().all(|h| h.view_ref.is_none()),
            "an inline view never gains a ref, so it reopens as the artifact it was"
        );
    }

    #[tokio::test]
    async fn history_is_bounded_and_drops_the_oldest() {
        let tmp = tempfile::tempdir().unwrap();
        let bus = ViewBus::load(tmp.path());
        for n in 0..HISTORY_MAX + 3 {
            bus.apply(show(&format!("v{n}"), &format!("/m/v{n}.mjs"))).await;
        }

        let state = bus.wait_state(None).await;
        assert_eq!(state.history.len(), HISTORY_MAX);
        assert_eq!(state.history.first().unwrap().id, "v3", "the oldest three fell off");
        assert_eq!(state.history.last().unwrap().id, format!("v{}", HISTORY_MAX + 2));
    }

    #[tokio::test]
    async fn a_show_only_ever_appends_so_nothing_a_person_was_returning_to_is_lost() {
        let tmp = tempfile::tempdir().unwrap();
        let bus = ViewBus::load(tmp.path());
        bus.apply(show("a", "/m/a.mjs")).await;
        bus.apply(show("b", "/m/b.mjs")).await;
        bus.apply(show("c", "/m/c.mjs")).await;
        // The person is parked on "a" — a cursor this bus never hears about. The
        // agent showing "d" must not disturb anything between there and the end.
        bus.apply(show("d", "/m/d.mjs")).await;

        let state = bus.wait_state(None).await;
        assert_eq!(
            history_ids(&state),
            vec!["a", "b", "c", "d"],
            "append-only: no forward entries were truncated by the new show"
        );
    }

    #[tokio::test]
    async fn the_label_is_the_refs_last_segment_opened_up() {
        let tmp = tempfile::tempdir().unwrap();
        let bus = ViewBus::load(tmp.path());
        bus.apply(show_ref("pr", "/m/pr.mjs", "factory/people-review")).await;
        bus.apply(show("bj01-final", "/m/x.mjs")).await;

        let state = bus.wait_state(None).await;
        assert_eq!(state.history[0].label, "People review");
        assert_eq!(state.history[1].label, "Bj01 final", "an inline view falls back to its id");
    }

    #[tokio::test]
    async fn history_survives_a_restart() {
        let tmp = tempfile::tempdir().unwrap();
        {
            let bus = ViewBus::load(tmp.path());
            bus.apply(show_ref("tasks", "/m/tasks.mjs", "factory/tasks")).await;
            bus.apply(show("bj01", "/m/bj01.mjs")).await;
        }

        let reloaded = ViewBus::load(tmp.path());
        let state = reloaded.wait_state(None).await;
        assert_eq!(
            history_ids(&state),
            vec!["tasks", "bj01"],
            "the way back is part of the state, so it comes back with it"
        );
        assert_eq!(state.history[0].view_ref.as_deref(), Some("factory/tasks"));
    }

    #[tokio::test]
    async fn the_condition_layer_never_enters_history() {
        let tmp = tempfile::tempdir().unwrap();
        let bus = ViewBus::load(tmp.path());
        bus.apply(show("tasks", "/m/tasks.mjs")).await;
        bus.reconcile(show("outage", "/m/outage.mjs")).await;

        let state = bus.wait_state(None).await;
        assert_eq!(ids(&state), vec!["tasks", "outage"], "layered over, as before");
        assert_eq!(
            history_ids(&state),
            vec!["tasks"],
            "an outage is a process condition, not somewhere the person was taken"
        );
    }

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
                view_ref: None,
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
            owns_conversation: true,
        };

        let version = {
            let bus = ViewBus::load(tmp.path());
            bus.apply(
                ViewEnvelope {
                    id: "g".into(),
                    op: ViewOp::Show,
                    module_url: Some("/m/g.mjs".into()),
                    traits: Some(traits),
                    view_ref: None,
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

    // ── restoring a view by its ref ───────────────────────────────────────────
    //
    // A compiler with no usable esbuild is enough for these: `compile` returns the
    // cache-hit URL without spawning anything when the module is already on disk,
    // and never gets called at all when the ref fails to resolve. `seed_view`
    // therefore writes the source AND its compiled artifact, which is exactly the
    // state a real boot is in after `install_factory_views` reseeds a view whose
    // module was compiled on an earlier run.

    fn offline_compiler(data_dir: &Path) -> crate::mind::views::ViewCompiler {
        crate::mind::views::ViewCompiler::new(PathBuf::from("/nonexistent/esbuild"), data_dir)
    }

    /// Write `source` at `views/<view_ref>.jsx` plus its compiled module, and
    /// answer with the URL that source compiles to.
    async fn seed_view(data_dir: &Path, view_ref: &str, source: &str) -> String {
        let views = data_dir.join("views");
        let path = views.join(format!("{view_ref}.jsx"));
        tokio::fs::create_dir_all(path.parent().unwrap()).await.unwrap();
        tokio::fs::write(&path, source).await.unwrap();

        let (hash, url) = crate::mind::views::module_ref(source);
        let compiled = views.join("_compiled");
        tokio::fs::create_dir_all(&compiled).await.unwrap();
        tokio::fs::write(compiled.join(format!("{hash}.mjs")), "// compiled").await.unwrap();
        url
    }

    fn show_ref(id: &str, url: &str, view_ref: &str) -> ViewEnvelope {
        ViewEnvelope {
            id: id.into(),
            op: ViewOp::Show,
            module_url: Some(url.into()),
            traits: None,
            view_ref: Some(view_ref.into()),
        }
    }

    /// The defect this whole path exists for: the source moved on while the screen
    /// held the module compiled from the *old* source. A restart must show the view
    /// as it is now.
    #[tokio::test]
    async fn refresh_recompiles_a_restored_view_from_its_current_source() {
        let tmp = tempfile::tempdir().unwrap();
        let stale = seed_view(tmp.path(), "deck/leader", "export default () => 'v1'").await;
        {
            let bus = ViewBus::load(tmp.path());
            bus.apply(show_ref("deck", &stale, "deck/leader")).await;
        }

        // The source is edited between runs — a view rebuild, or a new binary
        // reseeding `factory/`.
        let fresh = seed_view(tmp.path(), "deck/leader", "export default () => 'v2'").await;
        assert_ne!(stale, fresh, "the edit must change the content hash");

        let bus = ViewBus::load(tmp.path());
        assert_eq!(
            bus.wait_state(None).await.views[0].module_url, stale,
            "load alone restores the module the view was shown as"
        );

        bus.refresh_sources(&offline_compiler(tmp.path())).await;
        assert_eq!(
            bus.wait_state(None).await.views[0].module_url, fresh,
            "the refresh puts the view's current source on screen"
        );
    }

    /// Snapshots written before the ref existed pin a module and nothing else. The
    /// built-ins are the ones that go stale on their own (a binary reseeds them), and
    /// the agent shows each under its bare name — so an id matching a seeded
    /// `factory/` source identifies the view, and the refreshed snapshot records a
    /// real ref so the bridge is never needed again.
    #[tokio::test]
    async fn refresh_adopts_the_builtin_ref_for_a_snapshot_written_without_one() {
        let tmp = tempfile::tempdir().unwrap();
        seed_view(tmp.path(), "factory/tasks", "export default () => 'old'").await;
        {
            let bus = ViewBus::load(tmp.path());
            // No ref — exactly what a pre-`view_ref` snapshot restores as.
            bus.apply(show("tasks", "/views/_compiled/deadbeef.mjs")).await;
        }

        let current = seed_view(tmp.path(), "factory/tasks", "export default () => 'new'").await;
        let bus = ViewBus::load(tmp.path());
        bus.refresh_sources(&offline_compiler(tmp.path())).await;
        assert_eq!(bus.wait_state(None).await.views[0].module_url, current);

        // …and the ref is now on disk, so the next boot resolves it outright.
        let reloaded = ViewBus::load(tmp.path());
        let restored = reloaded.inner.lock().await.content.clone().unwrap();
        assert_eq!(restored.view_ref.as_deref(), Some("factory/tasks"));
    }

    /// An inline-source view has no durable name and no `factory/` file behind its
    /// id. It must be left exactly as it was rather than blanked.
    #[tokio::test]
    async fn refresh_leaves_a_view_it_cannot_name_alone() {
        let tmp = tempfile::tempdir().unwrap();
        {
            let bus = ViewBus::load(tmp.path());
            bus.apply(show("one-off", "/views/_compiled/abc123.mjs")).await;
        }

        let bus = ViewBus::load(tmp.path());
        bus.refresh_sources(&offline_compiler(tmp.path())).await;
        let state = bus.wait_state(None).await;
        assert_eq!(state.views[0].module_url, "/views/_compiled/abc123.mjs");
        assert_eq!(state.views[0].id, "one-off");
    }

    /// A ref whose source has since been deleted keeps the module it was shown as.
    /// A stale view beats an empty room, and the person can still clear it.
    #[tokio::test]
    async fn refresh_keeps_the_pinned_module_when_the_source_is_gone() {
        let tmp = tempfile::tempdir().unwrap();
        let url = seed_view(tmp.path(), "deck/leader", "export default () => 'v1'").await;
        {
            let bus = ViewBus::load(tmp.path());
            bus.apply(show_ref("deck", &url, "deck/leader")).await;
        }
        tokio::fs::remove_file(tmp.path().join("views/deck/leader.jsx")).await.unwrap();

        let bus = ViewBus::load(tmp.path());
        bus.refresh_sources(&offline_compiler(tmp.path())).await;
        assert_eq!(bus.wait_state(None).await.views[0].module_url, url);
    }

    /// The condition slot is host-owned and re-derived from embedded source every
    /// boot, so the refresh must not touch it.
    #[tokio::test]
    async fn refresh_leaves_the_condition_slot_to_its_own_reconcile() {
        let tmp = tempfile::tempdir().unwrap();
        {
            let bus = ViewBus::load(tmp.path());
            bus.reconcile(show("vendor-outage", "/m/energy.mjs")).await;
        }

        let bus = ViewBus::load(tmp.path());
        bus.refresh_sources(&offline_compiler(tmp.path())).await;
        let state = bus.wait_state(None).await;
        assert_eq!(ids(&state), vec!["vendor-outage"]);
        assert_eq!(state.views[0].module_url, "/m/energy.mjs");
    }
}
