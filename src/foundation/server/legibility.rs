//! `GET /api/legibility?days=7` — the numbers on what a person read, one set per surface
//! (`docs/arch/legibility.md` § I). It was `/api/speech` while speech was the only surface
//! anything judged.
//!
//! Derived on read from the judges' records ([`crate::mind::memory::quality`]), like
//! `/api/stats` is from the frame logs: a counter kept beside them would be free to drift.
//! Server-side and for whoever asks — nothing in the face renders it.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use chrono::{Duration, Utc};
use serde::Deserialize;

use super::AppState;
use crate::mind::memory::quality;

/// The window when none is asked for: a week is enough to read a trend and short enough
/// that last month's prompt is not averaged into this one's.
const DEFAULT_DAYS: i64 = 7;

#[derive(Debug, Deserialize, Default)]
pub struct LegibilityQuery {
    days: Option<i64>,
}

pub async fn get_legibility(
    State(state): State<Arc<AppState>>,
    Query(query): Query<LegibilityQuery>,
) -> Response {
    let days = query.days.unwrap_or(DEFAULT_DAYS).clamp(1, 366);
    let since = Utc::now() - Duration::days(days);
    let records = quality::read_since(&state.data_dir, since).await;
    let mut numbers = quality::legibility(&records);
    // The ledger's half of whether what the agent made reaches the person, read off the same
    // window of lines (`docs/arch/showing.md` § *Measurement*).
    numbers.showing.evidence = match crate::mind::memory::tasks::evidence(&state.data_dir, since).await {
        Ok(evidence) => Some(evidence),
        Err(error) => {
            tracing::warn!(%error, "the ledger could not be read for the showing numbers");
            None
        }
    };
    Json(numbers).into_response()
}
