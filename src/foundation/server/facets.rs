//! Memory-review endpoints — read the derived store, and repair one facet by hand.
//!
//! `docs/arch/data.md` calls facets "growable, revisable, correctable by one sentence
//! from the person", and says the correction path runs through the agent: the person
//! says it, the agent rewrites the facet, and "a correction that does not stick is now
//! a memory bug to fix, not a file to hand-edit." That holds — and it leaves two gaps
//! that only a surface can close. Nothing shows a person **what memory currently holds
//! about them**, so a wrong belief stays invisible until it produces a wrong answer;
//! and when a correction *doesn't* stick, there is no way to see that it didn't, or to
//! put the file right while the underlying bug is fixed. These routes close both:
//!
//! - `GET /api/facets` — every dimension on disk with its subjects.
//! - `GET /api/facets/{dimension}/{subject}` — one facet's raw markdown.
//! - `PUT /api/facets/{dimension}/{subject}` — replace it wholesale.
//! - `GET /api/episodes?limit=N` — the recent episode gists, newest first, as the
//!   evidence a facet was derived from.
//!
//! Facets regenerate from episodes at reflection time, so a hand-edit is not durable
//! truth: the next reflection re-derives the subject and may overwrite it. That is the
//! same last-writer-wins the store already accepts ([`facets`] "Concurrency"), and it
//! is why the episode list sits beside the facet list — a belief that keeps coming back
//! wrong is a claim about its episodes, and the reviewer needs to see both.
//!
//! Dimensions are open-ended (`people/`, `projects/`, `topics/`, `tasks/`), so the list
//! is a directory walk, never an enum. `tasks` is included even though it has its own
//! dedicated endpoint: a browser that hid part of the tree would misreport what memory
//! holds. Whether to render it differently is the view's call, not this one's.
//!
//! Writes go through [`facets::update_facet`], which renames a temp sibling into place,
//! so a reader never sees a torn file. Both path segments are checked for traversal and
//! then [`facets::slug`]ged, so a request can only ever name a path inside
//! `<data_dir>/memory/facets/`. The store is global, so like
//! `/api/people` these take no `X-HI-Conversation`.

use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

use axum::Json;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use crate::foundation::server::AppState;
use crate::mind::memory::{facets, layout};

/// Most episodes one `GET /api/episodes` will read back, and the default when the
/// caller names no limit. The cap is what keeps an unbounded review call from reading
/// the whole episode tree; the default is one screenful of scrollback.
const EPISODES_DEFAULT: usize = 50;
const EPISODES_MAX: usize = 500;

/// Whether one path segment may be used as a facet dimension/subject. Rejects the
/// traversal spellings outright rather than letting [`facets::slug`] quietly rewrite
/// them: `..` slugs to the empty string and `../evil` slugs to `evil`, both of which
/// stay inside the tree but answer a *different* question than the caller asked. A
/// review surface that silently retargeted an edit would be worse than one that 400s.
fn safe_segment(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty() && t != "." && t != ".." && !t.contains('/') && !t.contains('\\')
}

/// A filesystem timestamp as RFC3339 with a `Z` suffix (`2026-08-04T09:31:00Z`) — the
/// one timestamp spelling every JSON field here uses, so a view can compare/sort them
/// as plain strings. Empty when the platform has no mtime for the file.
fn stamp(t: Option<SystemTime>) -> String {
    match t {
        Some(t) => DateTime::<Utc>::from(t).to_rfc3339_opts(SecondsFormat::Secs, true),
        None => String::new(),
    }
}

/// Normalize a stored RFC3339 timestamp to the same `Z` spelling as [`stamp`]. Episode
/// frontmatter records `chrono`'s `to_rfc3339()` (`…+00:00`); a value that doesn't
/// parse is passed through verbatim rather than dropped — a debug surface shows what is
/// on disk.
fn normalize_ts(s: String) -> String {
    match DateTime::parse_from_rfc3339(&s) {
        Ok(d) => d.with_timezone(&Utc).to_rfc3339_opts(SecondsFormat::Secs, true),
        Err(_) => s,
    }
}

// ── list dimensions ───────────────────────────────────────────────────────────

#[derive(Serialize)]
struct DimensionDto {
    dimension: String,
    /// `subjects.len()`, carried explicitly so the view can show a count without
    /// walking the list.
    count: usize,
    subjects: Vec<String>,
}

/// Group the store's `<dim>/<subject>` refs by dimension. Reuses
/// [`facets::facet_subject_index`], so "exists" means the same thing here as it does to
/// reflection: the subject dir holds prose. A `people/` dir carrying only biometric
/// galleries is a cluster the mind hasn't modeled yet and belongs to `/api/people`, not
/// here. A missing facets dir yields an empty list — a fresh install has no facets and
/// that is not an error.
async fn list_dimensions(data_dir: &Path) -> anyhow::Result<Vec<DimensionDto>> {
    let refs = facets::facet_subject_index(data_dir).await?;
    let mut out: Vec<DimensionDto> = Vec::new();
    for r in refs {
        // The index is sorted, so equal dimensions arrive consecutively and the last
        // entry is the only one that can match.
        let Some((dim, subj)) = r.split_once('/') else {
            continue;
        };
        match out.last_mut() {
            Some(d) if d.dimension == dim => d.subjects.push(subj.to_owned()),
            _ => out.push(DimensionDto {
                dimension: dim.to_owned(),
                count: 0,
                subjects: vec![subj.to_owned()],
            }),
        }
    }
    for d in &mut out {
        d.count = d.subjects.len();
    }
    Ok(out)
}

/// `GET /api/facets` — every dimension on disk with its subjects, both sorted.
pub async fn get_facets(State(state): State<Arc<AppState>>) -> Response {
    match list_dimensions(&state.data_dir).await {
        Ok(dimensions) => Json(serde_json::json!({ "dimensions": dimensions })).into_response(),
        Err(e) => err(&e.to_string()),
    }
}

// ── read one facet ────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct FacetDto {
    /// The canonical (slugged) dimension the content was read from — not necessarily
    /// what the caller spelled.
    dimension: String,
    subject: String,
    /// The facet's raw markdown, frontmatter and all. No parsing: this surface shows
    /// ground truth so an edit round-trips byte-for-byte.
    content: String,
    bytes: usize,
    modified: String,
}

async fn read_one(data_dir: &Path, dim: &str, subject: &str) -> anyhow::Result<Option<FacetDto>> {
    let Some(content) = facets::read_facet(data_dir, dim, subject).await? else {
        return Ok(None);
    };
    let path = facets::subject_dir(data_dir, dim, subject).join(facets::FACET_FILE);
    let modified = stamp(tokio::fs::metadata(&path).await.ok().and_then(|m| m.modified().ok()));
    Ok(Some(FacetDto {
        dimension: facets::slug(dim),
        subject: facets::slug(subject),
        bytes: content.len(),
        content,
        modified,
    }))
}

/// `GET /api/facets/{dimension}/{subject}` — one facet's raw markdown. 404 when the
/// subject has no prose yet.
pub async fn get_facet(
    State(state): State<Arc<AppState>>,
    AxumPath((dimension, subject)): AxumPath<(String, String)>,
) -> Response {
    if !safe_segment(&dimension) || !safe_segment(&subject) {
        return err("dimension and subject must each be one path segment");
    }
    match read_one(&state.data_dir, &dimension, &subject).await {
        Ok(Some(dto)) => Json(dto).into_response(),
        Ok(None) => not_found("no facet for that dimension/subject"),
        Err(e) => err(&e.to_string()),
    }
}

// ── replace one facet ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct UpdateReq {
    /// The whole new facet body. Facets are regenerated, never patched, so this
    /// replaces the file — there is no partial-edit verb.
    content: String,
}

/// Write `content` as the whole facet and re-stat it, so the response reports the file
/// as it now stands rather than what the caller sent.
async fn write_one(
    data_dir: &Path,
    dim: &str,
    subject: &str,
    content: &str,
) -> anyhow::Result<(usize, String)> {
    facets::update_facet(data_dir, dim, subject, content).await?;
    let path = facets::subject_dir(data_dir, dim, subject).join(facets::FACET_FILE);
    let modified = stamp(tokio::fs::metadata(&path).await.ok().and_then(|m| m.modified().ok()));
    Ok((content.len(), modified))
}

/// `PUT /api/facets/{dimension}/{subject}` — replace a facet wholesale. Creates the
/// subject if it doesn't exist yet. Atomic via [`facets::update_facet`] (temp sibling +
/// rename), so a concurrent reader sees the old file or the new one, never a torn one.
pub async fn put_facet(
    State(state): State<Arc<AppState>>,
    AxumPath((dimension, subject)): AxumPath<(String, String)>,
    Json(req): Json<UpdateReq>,
) -> Response {
    if !safe_segment(&dimension) || !safe_segment(&subject) {
        return err("dimension and subject must each be one path segment");
    }
    match write_one(&state.data_dir, &dimension, &subject, &req.content).await {
        Ok((bytes, modified)) => {
            Json(serde_json::json!({ "ok": true, "bytes": bytes, "modified": modified }))
                .into_response()
        }
        Err(e) => err(&e.to_string()),
    }
}

// ── recent episodes ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct EpisodesQuery {
    limit: Option<usize>,
}

#[derive(Serialize)]
struct EpisodeDto {
    /// When the episode's covered range *ended* (`to_ts`), so the list orders by when
    /// something finished happening. Empty when the frontmatter carries neither
    /// `to_ts` nor `from_ts`.
    at: String,
    /// The episode body with its frontmatter stripped — the gist the mind wrote.
    gist: String,
}

/// The most recent `limit` episodes, newest first. Walks
/// `memory/episodes/<date>-<slug>/episode.md` directly rather than going through
/// [`crate::mind::memory::episodes::recent_gists`], which returns bodies only (no
/// timestamp) and oldest-first — this surface needs both fields.
/// A missing episodes dir, or one whose entries are unreadable, yields an empty list.
async fn recent_episodes(data_dir: &Path, limit: usize) -> anyhow::Result<Vec<EpisodeDto>> {
    let dir = layout::episodes_dir(data_dir);
    let mut rd = match tokio::fs::read_dir(&dir).await {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    // Sort on (at, dir name): `at` is a `Z`-normalized RFC3339 string, so lexical order
    // is chronological, and the dir name (`<date>-<slug>`) breaks ties deterministically
    // — including for an episode whose frontmatter carries no timestamp, which sorts to
    // the end rather than jumping the queue.
    let mut rows: Vec<(String, String, EpisodeDto)> = Vec::new();
    while let Some(ent) = rd.next_entry().await? {
        if !ent.file_type().await?.is_dir() {
            continue;
        }
        let Ok(name) = ent.file_name().into_string() else {
            continue;
        };
        let content = match tokio::fs::read_to_string(ent.path().join("episode.md")).await {
            Ok(c) => c,
            Err(_) => continue,
        };
        let at = frontmatter_field(&content, "to_ts")
            .or_else(|| frontmatter_field(&content, "from_ts"))
            .map(normalize_ts)
            .unwrap_or_default();
        let dto = EpisodeDto {
            gist: strip_frontmatter(&content).trim().to_owned(),
            at: at.clone(),
        };
        rows.push((at, name, dto));
    }
    rows.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    Ok(rows.into_iter().rev().take(limit).map(|(_, _, dto)| dto).collect())
}

/// `GET /api/episodes?limit=N` — recent episode gists, newest first. `limit` defaults to
/// 50 and is clamped to 500; the review view pairs these with a facet as the evidence it
/// was derived from.
pub async fn get_episodes(
    State(state): State<Arc<AppState>>,
    Query(q): Query<EpisodesQuery>,
) -> Response {
    let limit = q.limit.unwrap_or(EPISODES_DEFAULT).clamp(1, EPISODES_MAX);
    match recent_episodes(&state.data_dir, limit).await {
        Ok(episodes) => Json(serde_json::json!({ "episodes": episodes })).into_response(),
        Err(e) => err(&e.to_string()),
    }
}

// ── frontmatter ───────────────────────────────────────────────────────────────
//
// Duplicated from `mind::memory::episodes`, whose versions are `pub(super)` — visible
// only inside `mind::memory`. Same semantics, deliberately: split on the first `:` so an
// RFC3339 value keeps its own colons, JSON-decode a quoted value, and skip (not stop at)
// a line with no `:`. If a third reader ever appears, widen the originals to
// `pub(crate)` and delete these two.

/// One frontmatter scalar by key, or `None` if there is no frontmatter block or the key
/// is absent.
fn frontmatter_field(content: &str, key: &str) -> Option<String> {
    let fm = content.strip_prefix("---\n")?;
    let block = &fm[..fm.find("\n---\n")?];
    for line in block.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        if k.trim() != key {
            continue;
        }
        let v = v.trim();
        if v.starts_with('"')
            && let Ok(s) = serde_json::from_str::<String>(v)
        {
            return Some(s);
        }
        return Some(v.to_string());
    }
    None
}

/// Strip a leading `---\n…\n---\n` frontmatter block, returning the body.
fn strip_frontmatter(content: &str) -> &str {
    let Some(rest) = content.strip_prefix("---\n") else {
        return content;
    };
    match rest.find("\n---\n") {
        Some(i) => &rest[i + "\n---\n".len()..],
        None => content,
    }
}

/// A uniform JSON error body with a 400.
fn err(msg: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": msg }))).into_response()
}

/// The same body shape as [`err`] with a 404, so a client parses one error shape.
fn not_found(msg: &str) -> Response {
    (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": msg }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write an episode bundle the way `record_episode` does — same dir shape, same
    /// JSON-scalar frontmatter — so the parser is tested against the real on-disk form
    /// without standing up a journal.
    async fn write_episode(data_dir: &Path, name: &str, conversation: &str, to_ts: &str, gist: &str) {
        let dir = layout::episodes_dir(data_dir).join(name);
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let body = format!(
            "---\ntitle: {}\nfrom_id: \"a\"\nto_id: \"b\"\nfrom_ts: {}\nto_ts: {}\nsubjects: []\nkind: reflection\n---\n\n{gist}\n",
            serde_json::to_string(name).unwrap(),
            serde_json::to_string(to_ts).unwrap(),
            serde_json::to_string(to_ts).unwrap(),
        );
        tokio::fs::write(dir.join("episode.md"), body).await.unwrap();
    }

    #[tokio::test]
    async fn fresh_install_lists_nothing() {
        let d = tempfile::tempdir().unwrap();
        assert!(list_dimensions(d.path()).await.unwrap().is_empty());
        assert!(recent_episodes(d.path(), 50).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn list_groups_by_dimension_with_counts() {
        let d = tempfile::tempdir().unwrap();
        facets::update_facet(d.path(), "people", "Bob", "x").await.unwrap();
        facets::update_facet(d.path(), "people", "Alice", "x").await.unwrap();
        facets::update_facet(d.path(), "topics", "Kyoto Trip", "x").await.unwrap();
        // `tasks` is a dimension like any other and must show up in the browser.
        facets::update_facet(d.path(), "tasks", "Daily Digest", "x").await.unwrap();

        let dims = list_dimensions(d.path()).await.unwrap();
        assert_eq!(
            dims.iter().map(|x| x.dimension.as_str()).collect::<Vec<_>>(),
            ["people", "tasks", "topics"]
        );
        assert_eq!(dims.iter().map(|x| x.count).collect::<Vec<_>>(), [2, 1, 1]);
        assert_eq!(dims[0].subjects, vec!["alice".to_string(), "bob".to_string()]);
        assert_eq!(dims[1].subjects, vec!["daily-digest".to_string()]);
        assert_eq!(dims[2].subjects, vec!["kyoto-trip".to_string()]);
    }

    #[tokio::test]
    async fn read_update_read_roundtrips() {
        let d = tempfile::tempdir().unwrap();
        assert!(read_one(d.path(), "people", "alice").await.unwrap().is_none());

        facets::update_facet(d.path(), "people", "Alice", "Likes tea.\n").await.unwrap();
        let got = read_one(d.path(), "people", "alice").await.unwrap().unwrap();
        assert_eq!(got.dimension, "people");
        assert_eq!(got.subject, "alice");
        assert_eq!(got.content, "Likes tea.\n");
        assert_eq!(got.bytes, "Likes tea.\n".len());
        assert!(got.modified.ends_with('Z'), "modified was {:?}", got.modified);

        // The correction sticks: the next read is the corrected text, byte for byte.
        let fixed = "Likes coffee, not tea.\n";
        let (bytes, modified) = write_one(d.path(), "people", "alice", fixed).await.unwrap();
        assert_eq!(bytes, fixed.len());
        assert!(modified.ends_with('Z'));
        let got = read_one(d.path(), "people", "alice").await.unwrap().unwrap();
        assert_eq!(got.content, fixed);

        // A PUT to a subject that doesn't exist yet creates it.
        write_one(d.path(), "topics", "Tea", "Loose leaf.\n").await.unwrap();
        let dims = list_dimensions(d.path()).await.unwrap();
        assert_eq!(
            dims.iter().map(|x| x.dimension.as_str()).collect::<Vec<_>>(),
            ["people", "topics"]
        );
    }

    #[tokio::test]
    async fn traversal_segments_are_rejected() {
        let d = tempfile::tempdir().unwrap();
        for bad in ["..", ".", "../etc", "..\\etc", "a/b", "", "   "] {
            assert!(!safe_segment(bad), "{bad:?} should be rejected");
        }
        assert!(safe_segment("alice") && safe_segment("Kyoto Trip") && safe_segment("小雨"));

        // Belt and braces: even if a caller reached the writer, `..` slugs to nothing
        // and the store refuses it, and a dotted subject stays inside the facets tree.
        assert!(write_one(d.path(), "people", "..", "x").await.is_err());
        write_one(d.path(), "people", "...evil", "x").await.unwrap();
        assert!(
            facets::subject_dir(d.path(), "people", "...evil")
                .starts_with(layout::facets_dir(d.path()))
        );
    }

    #[tokio::test]
    async fn episodes_are_newest_first_and_capped() {
        let d = tempfile::tempdir().unwrap();
        let d0 = "2026-08-01T09:00:00+00:00";
        let d1 = "2026-08-02T10:00:00+00:00";
        let d2 = "2026-08-04T11:00:00+00:00";
        write_episode(d.path(), "2026-08-01-first", "boss", d0, "first").await;
        write_episode(d.path(), "2026-08-04-third", "boss", d2, "third").await;
        write_episode(d.path(), "2026-08-02-second", "alice@phone", d1, "second").await;

        let eps = recent_episodes(d.path(), 50).await.unwrap();
        assert_eq!(
            eps.iter().map(|e| e.gist.as_str()).collect::<Vec<_>>(),
            ["third", "second", "first"]
        );
        assert_eq!(eps[0].at, "2026-08-04T11:00:00Z");

        let eps = recent_episodes(d.path(), 1).await.unwrap();
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].gist, "third");
    }
}
