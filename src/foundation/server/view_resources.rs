//! Named resources of a view. The binding file contains references, never image bytes.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::foundation::attachments::{self, Kind};
use crate::foundation::server::AppState;

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 80
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn binding_path(data_dir: &Path, view_ref: &str) -> std::path::PathBuf {
    data_dir.join("views").join(format!("{view_ref}.resources.json"))
}

pub async fn bindings(data_dir: &Path, view_ref: &str) -> Result<BTreeMap<String, String>, String> {
    if !crate::mind::views::valid_ref(view_ref) {
        return Err("invalid view ref".into());
    }
    let path = binding_path(data_dir, view_ref);
    let raw = match tokio::fs::read_to_string(&path).await {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(format!("reading {}: {e}", path.display())),
    };
    let refs: BTreeMap<String, String> =
        serde_json::from_str(&raw).map_err(|e| format!("invalid {}: {e}", path.display()))?;
    for (name, reference) in &refs {
        if !valid_name(name) || attachments::ref_id(reference).is_none() {
            return Err(format!("invalid resource {name} in {}", path.display()));
        }
        if attachments::probe(data_dir, attachments::ref_id(reference).unwrap()).await.is_none() {
            return Err(format!("missing attachment {reference} in {}", path.display()));
        }
    }
    Ok(refs)
}

/// The route is owned by this view, even though the object lives in the shared store.
pub async fn serve(state: Arc<AppState>, view_ref: &str, name: &str, spec: &str, req: Request) -> Response {
    let Ok(refs) = bindings(&state.data_dir, view_ref).await else {
        return (StatusCode::NOT_FOUND, "invalid view resources").into_response();
    };
    let Some(id) = refs.get(name).and_then(|r| attachments::ref_id(r)) else {
        return (StatusCode::NOT_FOUND, "no such view resource").into_response();
    };
    let base = format!("/views/{view_ref}.assets/_resources/{name}");
    if spec == attachments::ABOUT_SPEC {
        let Some(probe) = attachments::probe(&state.data_dir, id).await else {
            return StatusCode::NOT_FOUND.into_response();
        };
        let mut info = attachments::about(id, &probe);
        info["url"] = format!("{base}/{}", if probe.kind == Kind::Clip { "playable" } else { "original" }).into();
        info["preview"] = format!("{base}/{}", attachments::PREVIEW_SPEC).into();
        let mut response = axum::Json(info).into_response();
        response.headers_mut().insert(axum::http::header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        return response;
    }
    if spec == "playable" {
        let Some(playable) = attachments::playable(&state.data_dir, id).await else {
            return StatusCode::NOT_FOUND.into_response();
        };
        let target = match playable {
            attachments::Playable::Original => "original",
            attachments::Playable::Copy(_) => "proxy.v1",
            attachments::Playable::Preparing => return (StatusCode::SERVICE_UNAVAILABLE, [(axum::http::header::RETRY_AFTER, "3")], "preparing").into_response(),
        };
        return (StatusCode::FOUND, [(axum::http::header::LOCATION, format!("{base}/{target}")), (axum::http::header::CACHE_CONTROL, "no-store".into())]).into_response();
    }
    let file = match spec {
        "original" => attachments::object(&state.data_dir, id).await.map(|(p, probe)| (p, probe.mime())),
        "proxy.v1" => match attachments::playable(&state.data_dir, id).await {
            Some(attachments::Playable::Copy(p)) => Some((p, "video/mp4")),
            _ => None,
        },
        attachments::PREVIEW_SPEC => attachments::preview(&state.data_dir, id).await,
        _ => None,
    };
    let Some((path, mime)) = file else { return StatusCode::NOT_FOUND.into_response() };
    let mut response = super::disk_file::serve(req, &path, mime, "no-store", "no such view resource").await;
    response.headers_mut().insert(axum::http::header::HeaderName::from_static("x-content-type-options"), HeaderValue::from_static("nosniff"));
    response
}
