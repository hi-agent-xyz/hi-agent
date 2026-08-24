//! GET /api/out/view — long-poll for the retained appearance state.
//!
//! Appearance is state, not a stream: the response is the whole set of
//! active views (z-ordered) plus a version, served by the [`ViewBus`]. A call
//! without `?since=` returns the current state immediately — even when empty —
//! so a fresh page syncs on open; passing the last seen version parks until
//! the state changes. The reaction mutates the state when the agent calls
//! `show` and the view compiler has turned its source into a module.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::IntoResponse;

use chrono::Utc;
use uuid::Uuid;

use crate::foundation::server::AppState;
use crate::foundation::server::headers::AuthBearer;
use crate::types::{Channel, JournalEntry, Origin, Sender};

#[derive(serde::Deserialize)]
pub struct ViewQuery {
    /// The last appearance version this client has rendered; the response is
    /// held until the appearance version exceeds it.
    since: Option<u64>,
}

pub async fn get_out_view(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ViewQuery>,
    AuthBearer(auth): AuthBearer,
) -> impl IntoResponse {
    // A held view long-poll = a screen is attached; counted until this handler returns.
    let _attached = state.attachments.connect(crate::body::attachments::OutChannel::View);

    tracing::info!(since = ?query.since, auth = ?auth, "GET /api/out/view long-poll opened");

    // Opening this long-poll is a presence signal: warm up so the
    // process + session + upstream cache are hot before the first utterance.
    state.warm();

    axum::Json(state.views.wait_state(query.since).await)
}

/// DELETE /api/out/view — clear the appearance (close all views, back
/// to the default empty room). A user control: the screen is the agent's
/// presentation, but the user can reclaim it. The clear bumps the version, so
/// every device's long-poll converges on the empty state. Returns 204.
pub async fn clear_out_view(
    State(state): State<Arc<AppState>>,
    AuthBearer(auth): AuthBearer,
) -> impl IntoResponse {
    tracing::info!(auth = ?auth, "DELETE /api/out/view — user cleared the screen");
    state.views.clear().await;
    axum::http::StatusCode::NO_CONTENT
}

/// One view that exists on disk and can be opened by name.
#[derive(serde::Serialize)]
pub struct ListedView {
    /// The durable ref, e.g. `factory/drive`.
    pub view_ref: String,
    pub label: String,
    /// A system surface — one of the views we ship, under `factory/`. These are the
    /// standing floor of the bookmarks row: always there, and not removable, because
    /// they are how a person reaches their tasks, their memory and their files at all.
    pub system: bool,
    /// The person put this one in the row. Always false for a system view, which is
    /// in the row by being system.
    pub bookmarked: bool,
    /// A picture of this surface as it currently stands, served from
    /// `/views/_shots/ref/<ref>.png`. Absent until one has been taken — see
    /// [`super::view_shots`] for when that is. It is what a card the person opens
    /// carries into the band's row, so a view they went to has a face even though the
    /// agent never shown it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shot_url: Option<String>,
}

/// How many first pictures one inventory read may start rendering. The band is the only
/// caller and a person opens it a few times an hour, so this warms the shipped dozen
/// over a handful of opens instead of putting twelve Chromiums on the machine at the
/// moment someone reaches for their tasks. Missing pictures only: keeping one current is
/// the job of opening the view, which is also the only evidence anyone cares what is
/// on it.
const WARM_PER_READ: usize = 3;

/// Refs a warm-up is already working on. The band re-reads the inventory every few
/// seconds while it is open, and a picture takes longer than that to render, so without
/// this every read would queue the same three views again behind the ones already
/// rendering them.
static WARMING: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<String>>> =
    std::sync::LazyLock::new(Default::default);

/// `app_settings` key holding the person's bookmarked refs, as a JSON array.
///
/// **In the config store, not the views tree.** The tree is disposable and re-seeded
/// on every boot; a bookmark is one of the few things in this product the *person*
/// wrote, and it has to outlive an upgrade that replaces `factory/` wholesale. It is
/// server-side rather than per-window for the reason the design gives for not having
/// built it sooner: it is state that must be the same on the desktop and the phone.
const BOOKMARKS_KEY: &str = "view_bookmarks";

/// The prefix a system view's ref carries. Same constant the factory seeder writes
/// under; see [`crate::mind::views::factory`].
const SYSTEM_PREFIX: &str = "factory/";

/// The person's bookmarked refs. A store that cannot be read reads as none — the row
/// is then the system views alone, which is the state it ships in.
fn read_bookmarks(data_dir: &std::path::Path) -> Vec<String> {
    crate::foundation::credentials::get_setting(data_dir, BOOKMARKS_KEY)
        .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
        .unwrap_or_default()
}

/// `GET /api/views` — every named view in the views tree, alphabetically, each saying
/// whether it is a system surface and whether the person bookmarked it.
///
/// This is the inventory, and it exists because a dozen views shipped with no way to
/// reach any of them except asking the agent to show it. Asking for the drive every
/// time is the interaction cost of a chatbot sitting on top of what is otherwise an
/// app, and a person may go to a place even though the agent decides what to show.
///
/// **The whole inventory is no longer the row.** The design deferred a person-owned
/// subset on the grounds that a dozen views fit one row; what is actually in the tree
/// after a week of work is those dozen plus every one-off a builder ever wrote —
/// `entry`, `entry b`, `entry mlat`, `mount b` — and the row became a list of the
/// agent's scratch files with the surfaces a person actually wants buried in it. So
/// the endpoint still reports everything (the inventory is the truth about the tree)
/// and marks what belongs in the row: the system views, plus what the person kept.
///
/// `_compiled/` and `_shots/` are skipped — tool dirs inside the tree, like
/// `node_modules`, not views. So is the condition view: it is the host's, put up and
/// taken down by [`ViewBus::reconcile`](super::ViewBus::reconcile) against a live
/// process level, and offering it as a place a person can go would let them summon a
/// vendor outage that isn't happening.
pub async fn list_views(
    State(state): State<Arc<AppState>>,
    AuthBearer(auth): AuthBearer,
) -> impl IntoResponse {
    tracing::debug!(auth = ?auth, "GET /api/views");
    let root = state.data_dir.join("views");
    let mut found = Vec::new();
    collect_views(&root, &root, &mut found).await;
    let saved = read_bookmarks(&state.data_dir);
    for view in &mut found {
        view.system = view.view_ref.starts_with(SYSTEM_PREFIX);
        view.bookmarked = !view.system && saved.iter().any(|r| r == &view.view_ref);
        view.shot_url = super::view_shots::url_for_ref(&state.data_dir, &view.view_ref);
    }
    found.sort_by(|a: &ListedView, b: &ListedView| a.view_ref.cmp(&b.view_ref));

    // The row's own views, and only the ones with no picture at all. Reading the
    // inventory *is* the signal that the band is open, which is the one moment the
    // pictures are about to be looked at.
    let cold: Vec<String> = {
        let mut warming = WARMING.lock().unwrap_or_else(|held| held.into_inner());
        found
            .iter()
            .filter(|v| (v.system || v.bookmarked) && v.shot_url.is_none())
            .map(|v| v.view_ref.clone())
            .filter(|view_ref| warming.insert(view_ref.clone()))
            .take(WARM_PER_READ)
            .collect()
    };
    if !cold.is_empty() {
        tokio::spawn(warm_shots(state.clone(), cold));
    }
    axum::Json(found)
}

/// Take a first picture of each of `refs`, in the background.
///
/// Compiling is what makes this more than a render: a surface's picture has to be of
/// the view as it is now, so it goes through the same resolve-and-compile the person's
/// own open does. Everything here is best-effort — a view that no longer resolves or
/// compiles simply keeps the mark it already had, and the band never learns there was
/// an attempt.
async fn warm_shots(state: Arc<AppState>, refs: Vec<String>) {
    for view_ref in refs {
        if warm_one(&state, &view_ref).await {
            // Same bump a show's capture makes: the picture has to reach the windows
            // whose long-poll was answered before it existed.
            state.views.note_shot().await;
        }
        WARMING.lock().unwrap_or_else(|held| held.into_inner()).remove(&view_ref);
    }
}

/// One warm-up: resolve, compile, render. `true` if a picture landed.
async fn warm_one(state: &Arc<AppState>, view_ref: &str) -> bool {
    let Some(render) = crate::mind::views::render_context() else {
        return false;
    };
    let Ok(source) = crate::mind::views::resolve_ref(&state.data_dir, view_ref).await else {
        return false;
    };
    let Ok(module_url) = render.compiler.compile(&source).await else {
        return false;
    };
    super::view_shots::take_ref(&state.data_dir, view_ref, &module_url).await
}

#[derive(serde::Deserialize)]
pub struct BookmarkRequest {
    #[serde(rename = "ref")]
    pub view_ref: String,
    /// `true` puts it in the row, `false` takes it out.
    pub on: bool,
}

/// `POST /api/views/bookmarks` — keep a named view in the bookmarks row, or drop it.
///
/// **Only a named view can be bookmarked.** An inline view has no durable name; it is
/// only ever the content-addressed artifact it compiled to, and the compiled tree is a
/// disposable cache. A bookmark to one would be a bookmark to a hash that the next
/// prune deletes, so the answer is 400 rather than a link that quietly rots — the same
/// named/inline split that decides what re-opening means.
///
/// A system view is refused too, for the opposite reason: it is in the row by being a
/// system view, so a stored bookmark for it would be a second, disagreeing source of
/// truth about a row that is already showing it.
pub async fn bookmark_view(
    State(state): State<Arc<AppState>>,
    AuthBearer(auth): AuthBearer,
    axum::Json(body): axum::Json<BookmarkRequest>,
) -> impl IntoResponse {
    let view_ref = body.view_ref.trim().to_string();
    tracing::info!(auth = ?auth, view_ref = %view_ref, on = body.on, "POST /api/views/bookmarks");
    if !crate::mind::views::valid_ref(&view_ref) {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            "only a named view can be bookmarked".to_string(),
        )
            .into_response();
    }
    if view_ref.starts_with(SYSTEM_PREFIX) {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            "a system view is in the row already".to_string(),
        )
            .into_response();
    }
    let mut saved = read_bookmarks(&state.data_dir);
    if body.on {
        if !saved.iter().any(|r| r == &view_ref) {
            saved.push(view_ref);
        }
    } else {
        saved.retain(|r| r != &view_ref);
    }
    let encoded = serde_json::to_string(&saved).unwrap_or_else(|_| "[]".to_string());
    if let Err(error) =
        crate::foundation::credentials::set_setting(&state.data_dir, BOOKMARKS_KEY, &encoded)
    {
        tracing::warn!(%error, "storing the bookmarks failed");
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "could not store the bookmark".to_string(),
        )
            .into_response();
    }
    axum::http::StatusCode::NO_CONTENT.into_response()
}

/// Walk the views tree collecting `<rel>.jsx` as refs. Iterative rather than recursive
/// because an `async fn` cannot recurse without boxing, and the tree is shallow enough
/// that a worklist is the plainer of the two.
async fn collect_views(root: &std::path::Path, start: &std::path::Path, out: &mut Vec<ListedView>) {
    let mut queue = vec![start.to_path_buf()];
    while let Some(dir) = queue.pop() {
        let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
            continue;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
                // `_compiled/` and `_shots/` are the tool dirs; leading `_` is the rule
                // rather than a list, so the next one does not have to be remembered here.
                if !name.starts_with('_') {
                    queue.push(path);
                }
                continue;
            }
            if !name.ends_with(".jsx") {
                continue;
            }
            let Ok(rel) = path.strip_prefix(root) else {
                continue;
            };
            let view_ref = rel
                .with_extension("")
                .components()
                .map(|c| c.as_os_str().to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .join("/");
            if !crate::mind::views::valid_ref(&view_ref)
                || view_ref == crate::mind::views::factory::OUT_OF_ENERGY_REF
            {
                continue;
            }
            let label = crate::foundation::server::view_bus::humanize_ref(&view_ref);
            // `system`/`bookmarked` are decided by the caller, which is the only place
            // that has read the store.
            out.push(ListedView {
                view_ref,
                label,
                system: false,
                bookmarked: false,
                shot_url: None,
            });
        }
    }
}

#[derive(serde::Deserialize)]
pub struct OpenViewRequest {
    /// The durable ref to open, e.g. `factory/drive`.
    #[serde(rename = "ref")]
    pub view_ref: String,
}

/// A view compiled for the person to mount locally.
#[derive(serde::Serialize)]
pub struct OpenedView {
    pub id: String,
    pub module_url: String,
}

/// `POST /api/views/open` — compile a named view so this window can mount it, **without
/// touching the content slot**.
///
/// This is the person's path onto the screen and it is deliberately not a third writer
/// of the appearance. The slot stays the agent's — what it showed is still what a second
/// device shows and still what it will refer to out loud. Where *this* window is looking
/// is this window's own business, like the conversation's scroll position, and like that
/// it is never reported back.
///
/// It resolves the ref every time rather than handing back a remembered module, which is
/// what makes opening `factory/tasks` show today's board. An inline view has no ref and
/// cannot be opened this way at all; the client mounts its recorded artifact directly.
///
/// It also re-takes the surface's picture, which is how a view the agent never shown
/// gets a face in the band at all, and how the shipped surfaces stop showing the board
/// they had the first time anyone looked. Bounded by the same staleness rule every
/// other surface capture uses, so opening the same view five times is one render.
pub async fn open_view(
    State(state): State<Arc<AppState>>,
    AuthBearer(auth): AuthBearer,
    axum::Json(body): axum::Json<OpenViewRequest>,
) -> impl IntoResponse {
    let view_ref = body.view_ref.trim().to_string();
    tracing::info!(auth = ?auth, view_ref = %view_ref, "POST /api/views/open");

    let Some(render) = crate::mind::views::render_context() else {
        return (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "the view compiler is not up yet".to_string(),
        )
            .into_response();
    };
    let source = match crate::mind::views::resolve_ref(&state.data_dir, &view_ref).await {
        Ok(source) => source,
        Err(error) => {
            return (axum::http::StatusCode::NOT_FOUND, error).into_response();
        }
    };
    match render.compiler.compile(&source).await {
        Ok(module_url) => {
            // Going somewhere is the moment its picture is worth re-taking: the person
            // is looking at the board right now, so whatever the browser sees a second
            // later is what they saw. Off the response path — this returns before the
            // capture starts, and the band picks the picture up on its next read.
            let bus = state.views.clone();
            super::view_shots::capture_ref(
                state.data_dir.clone(),
                view_ref.clone(),
                module_url.clone(),
                move || {
                    tokio::spawn(async move { bus.note_shot().await });
                },
            );
            axum::Json(OpenedView { id: view_ref, module_url })
            .into_response()
        }
        Err(error) => (
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            format!("view does not compile: {error}"),
        )
            .into_response(),
    }
}

/// Where a window just went. Either `live`, or a destination named by `ref` (a named
/// view) or `module` (an inline one) — the same two-way split the history dedupes by.
#[derive(serde::Deserialize)]
pub struct WentToRequest {
    #[serde(default, rename = "ref")]
    pub view_ref: Option<String>,
    /// The compiled module URL, for an inline view that has no ref.
    #[serde(default)]
    pub module: Option<String>,
    /// What the view was shown as. Only used to name an inline destination, whose
    /// module hash names nothing.
    #[serde(default)]
    pub id: Option<String>,
    /// The person is on what the agent currently has up. Then there is nothing to tell
    /// the agent about where they are, and the fact is cleared rather than recorded.
    #[serde(default)]
    pub live: bool,
}

/// `POST /api/in/view` — the person went to a view.
///
/// The inbound half of a channel that used to only go out, and the one thing about the
/// screen that does not come from the agent. It exists because the next thing they say is
/// usually about what they are looking at: "这个数字不对" is unreadable if the agent
/// believes its own last show is in front of them, and it answers confidently about the
/// wrong board.
///
/// **It records where they went, never which window went there.** The cursor stays the
/// window's — a phone that went back must not move the desktop — so nothing here touches
/// the appearance: no version bump, no snapshot, and `GET /api/out/view` is unchanged.
/// This is a perception, not a third writer of the content slot.
///
/// **And it does not drive a turn.** Walking the band through five tiles must not produce
/// five turns; the move goes to the journal and to the view bus, and the next turn reads
/// it as context — which is the moment it matters. That is why there is no
/// `state.inbound.send` here, unlike every other `/api/in/*` handler.
pub async fn post_in_view(
    State(state): State<Arc<AppState>>,
    AuthBearer(auth): AuthBearer,
    axum::Json(body): axum::Json<WentToRequest>,
) -> impl IntoResponse {
    let view_ref = body.view_ref.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let module = body.module.as_deref().map(str::trim).filter(|s| !s.is_empty());
    // The destination, by the same rule the history dedupes by: the ref when there is
    // one, else the compiled module.
    let key = match (body.live, view_ref.or(module)) {
        (true, _) => String::new(),
        (false, Some(key)) => key.to_owned(),
        // A move to nowhere is a client bug, and answering 202 to it would hide the
        // bug behind a context line that silently never appears.
        (false, None) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                "a move needs a ref or a module, unless it is back to live",
            )
                .into_response();
        }
    };
    let name = view_ref
        .or(body.id.as_deref().map(str::trim).filter(|s| !s.is_empty()))
        .unwrap_or(&key)
        .to_owned();

    tracing::info!(auth = ?auth, dest = %name, live = body.live, "POST /api/in/view");

    // A re-click of the tile they are already on moved nobody, and the log is a record
    // of moves.
    if !state.views.note_went_to(key, name.clone(), body.live).await {
        return axum::http::StatusCode::ACCEPTED.into_response();
    }

    let line = if body.live {
        "came back to the live view".to_owned()
    } else {
        format!("went to \"{name}\"")
    };
    let ts = Utc::now();
    crate::foundation::channel_log::inbound(Channel::View, &line);

    // Addressed, like text: this is the person acting on the agent's own surface,
    // through a control nobody else can reach. Labelled `owner` rather than written
    // bare — see `docs/arch/signal-attribution.md`.
    let sender = Sender::owner_or_unknown(crate::foundation::config::tunables::owner().as_deref());
    // Perception, not conversation: the person walking the band is something the
    // agent noticed them do, read into the next turn's context rather than answered.
    let entry = JournalEntry::Observation {
        id: Uuid::now_v7().to_string(),
        ts,
        channel: Channel::View,
        body: line.clone(),
        stream: None,
        media: None,
        sender: Some(sender),
    };
    if let Err(err) = state.memory.journal.append(entry).await {
        tracing::warn!(error = %format!("{err:#}"), "view move: journal append failed");
    }

    // The inspector's live tap sees both halves of the channel now. Not the
    // conversation: `transcript::from_journal` drops `View` on both directions, because
    // going somewhere is not something anybody said.
    let _ = state.input_echo.send(crate::foundation::server::InputEcho {
        channel: Channel::View,
        text: line,
        is_final: true,
        ts,
    });

    axum::http::StatusCode::ACCEPTED.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bookmarks_round_trip_through_the_config_store() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_bookmarks(dir.path()).is_empty(), "a fresh core has none");

        crate::foundation::credentials::set_setting(
            dir.path(),
            BOOKMARKS_KEY,
            r#"["badminton-top10/leader","notes/board"]"#,
        )
        .unwrap();
        assert_eq!(
            read_bookmarks(dir.path()),
            vec!["badminton-top10/leader".to_string(), "notes/board".to_string()],
        );
    }

    /// The row is what a person reaches their work through, so a value that cannot be
    /// parsed has to degrade to the system views rather than to an error: they still
    /// get to their tasks, and the next write repairs the key.
    #[test]
    fn a_store_holding_nonsense_reads_as_no_bookmarks() {
        let dir = tempfile::tempdir().unwrap();
        crate::foundation::credentials::set_setting(dir.path(), BOOKMARKS_KEY, "not json")
            .unwrap();
        assert!(read_bookmarks(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn the_tool_dirs_are_not_places_a_person_can_go() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("views");
        for rel in ["factory", "_compiled", "_shots", "notes"] {
            std::fs::create_dir_all(root.join(rel)).unwrap();
        }
        std::fs::write(root.join("factory/tasks.jsx"), "x").unwrap();
        std::fs::write(root.join("notes/board.jsx"), "x").unwrap();
        // A compiled artifact is a `.mjs`, but a hand-dropped `.jsx` in either tool dir
        // must not become a destination either — the rule is the directory, not the
        // extension that happens to live in it today.
        std::fs::write(root.join("_compiled/deadbeef.jsx"), "x").unwrap();
        std::fs::write(root.join("_shots/deadbeef.jsx"), "x").unwrap();

        let mut found = Vec::new();
        collect_views(&root, &root, &mut found).await;
        let mut refs: Vec<String> = found.into_iter().map(|v| v.view_ref).collect();
        refs.sort();
        assert_eq!(refs, vec!["factory/tasks".to_string(), "notes/board".to_string()]);
    }
}
