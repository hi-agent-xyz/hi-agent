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

use crate::foundation::server::AppState;
use crate::foundation::server::headers::AuthBearer;

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
}

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
/// app, and a person may go to a place even though the agent decides what to raise.
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
    }
    found.sort_by(|a: &ListedView, b: &ListedView| a.view_ref.cmp(&b.view_ref));
    axum::Json(found)
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
            out.push(ListedView { view_ref, label, system: false, bookmarked: false });
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traits: Option<crate::types::ViewTraits>,
}

/// `POST /api/views/open` — compile a named view so this window can mount it, **without
/// touching the content slot**.
///
/// This is the person's path onto the screen and it is deliberately not a third writer
/// of the appearance. The slot stays the agent's — what it raised is still what a second
/// device shows and still what it will refer to out loud. Where *this* window is looking
/// is this window's own business, like the conversation's scroll position, and like that
/// it is never reported back.
///
/// It resolves the ref every time rather than handing back a remembered module, which is
/// what makes opening `factory/tasks` show today's board. An inline view has no ref and
/// cannot be opened this way at all; the client mounts its recorded artifact directly.
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
    let (source, traits) = match crate::mind::views::resolve_ref(&state.data_dir, &view_ref).await {
        Ok(resolved) => resolved,
        Err(error) => {
            return (axum::http::StatusCode::NOT_FOUND, error).into_response();
        }
    };
    match render.compiler.compile(&source).await {
        Ok(module_url) => axum::Json(OpenedView {
            id: view_ref,
            module_url,
            traits,
        })
        .into_response(),
        Err(error) => (
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            format!("view does not compile: {error}"),
        )
            .into_response(),
    }
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
