//! Same owner gate as the other review reads, including the remote tunnel.
use axum::{
    Json,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};

// Queue readers asynchronously, rather than exhausting the blocking pool with
// threads waiting for the shared sampler's lock.
static READER: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub async fn get_status() -> Response {
    let _reader = READER.lock().await;
    match tokio::task::spawn_blocking(crate::foundation::system_status::snapshot).await {
        Ok(snapshot) => ([(header::CACHE_CONTROL, "no-store")], Json(snapshot)).into_response(),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "System status unavailable").into_response(),
    }
}
