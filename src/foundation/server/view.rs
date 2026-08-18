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
}

/// `GET /api/views` — every named view in the views tree, alphabetically.
///
/// This is the inventory, and it exists because a dozen views shipped with no way to
/// reach any of them except asking the agent to show it. Asking for the drive every
/// time is the interaction cost of a chatbot sitting on top of what is otherwise an
/// app, and a person may go to a place even though the agent decides what to raise.
///
/// There is no person-owned pinned subset yet, deliberately: the tree holds about a
/// dozen views, they fit one scrolling row, and pinning is the only part of this that
/// would need new state syncing across devices. Add it when the row is long enough to
/// be a problem.
///
/// `_compiled/` is skipped — it is the disposable module cache, a tool dir inside the
/// tree like `node_modules`, not a view. So is the condition view: it is the host's,
/// put up and taken down by [`ViewBus::reconcile`](super::ViewBus::reconcile) against a
/// live process level, and offering it as a place a person can go would let them summon
/// a vendor outage that isn't happening.
pub async fn list_views(
    State(state): State<Arc<AppState>>,
    AuthBearer(auth): AuthBearer,
) -> impl IntoResponse {
    tracing::debug!(auth = ?auth, "GET /api/views");
    let root = state.data_dir.join("views");
    let mut found = Vec::new();
    collect_views(&root, &root, &mut found).await;
    found.sort_by(|a: &ListedView, b: &ListedView| a.view_ref.cmp(&b.view_ref));
    axum::Json(found)
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
                if name != "_compiled" {
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
            out.push(ListedView { view_ref, label });
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
