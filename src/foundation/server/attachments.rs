//! `GET /api/attachments/{id}` and `GET /api/attachments/{id}/{spec}` — an attachment's
//! bytes, and the picture every surface draws it by (`docs/arch/showing.md` § *Serving*).
//!
//! **Both are `immutable`, and both can say so honestly.** The id is the SHA-256 of the
//! bytes, so nothing different can ever be served at that path; the preview's URL carries
//! its spec, so a new way of drawing previews is a new URL rather than new bytes behind an
//! old one. That is what lets a browser keep either forever, and what marks both as
//! mirrorable off the machine (`docs/arch/topology.md` § *Content*).
//!
//! Served through [`disk_file`](super::disk_file) like the media and drive routes, so a clip
//! streams and seeks — WebKit opens a `<video>` by asking for `bytes=0-1`.
//!
//! **Nothing an attachment contains can run.** This store accepts pictures and clips only,
//! and every response says `nosniff`, so a file is drawn as what its probe says it is and
//! never as whatever a browser guesses.

use std::sync::Arc;

use axum::extract::{Path, Request, State};
use axum::http::header::HeaderName;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::foundation::attachments::{self, PREVIEW_SPEC};
use crate::foundation::server::AppState;

const IMMUTABLE: &str = "private, max-age=31536000, immutable";

pub async fn get_attachment(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    req: Request,
) -> Response {
    let Some((path, probe)) = attachments::object(&state.data_dir, &id).await else {
        return (StatusCode::NOT_FOUND, "no such attachment").into_response();
    };
    nosniff(super::disk_file::serve(req, &path, probe.mime(), IMMUTABLE, "no such attachment").await)
}

pub async fn get_derived(
    State(state): State<Arc<AppState>>,
    Path((id, spec)): Path<(String, String)>,
    req: Request,
) -> Response {
    if spec == attachments::STAGE_SPEC {
        return stage_module(&state, &id).await;
    }
    if spec != PREVIEW_SPEC {
        return (StatusCode::NOT_FOUND, "no such derivation").into_response();
    }
    let Some((path, mime)) = attachments::preview(&state.data_dir, &id).await else {
        return (StatusCode::NOT_FOUND, "no preview").into_response();
    };
    nosniff(super::disk_file::serve(req, &path, mime, IMMUTABLE, "no preview").await)
}

/// The module the stage mounts for an attachment ([`attachments::stage_module`]). Generated,
/// not read off disk, and as immutable as the object it names: the id fixes the probe, and the
/// spec in the path fixes what the module says.
async fn stage_module(state: &AppState, id: &str) -> Response {
    let (Some(id), Some(probe)) = (attachments::parse_id(id), attachments::probe(&state.data_dir, id).await)
    else {
        return (StatusCode::NOT_FOUND, "no such attachment").into_response();
    };
    let mut resp = attachments::stage_module(id, &probe).into_response();
    let headers = resp.headers_mut();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static("text/javascript; charset=utf-8"),
    );
    headers.insert(axum::http::header::CACHE_CONTROL, HeaderValue::from_static(IMMUTABLE));
    nosniff(resp)
}

fn nosniff(mut resp: Response) -> Response {
    resp.headers_mut().insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    resp
}
