//! Skill-workshop endpoints — the backend for the view that reads and prunes
//! `<data_dir>/skills/`.
//!
//! A skill is a plain `.md` note the agent wrote about how a kind of job was done
//! ([`crate::mind::skills`]). Nothing in the running system re-checks one: a worker
//! reads a skill and follows it. So a skill that has gone stale — a dead endpoint, a
//! price that moved, a step that no longer applies — silently poisons every later run
//! of that job. The user needs a way to see what is in the workshop and delete the
//! entry that is wrong; that is the whole point of this surface, and why `DELETE` is
//! here while no write verb is (the agent writes its own skills; the human corrects).
//!
//! - `GET    /api/skills` — the whole tree, learnt notes first, freshest first.
//! - `GET    /api/skills/{*path}` — one skill's raw markdown.
//! - `DELETE /api/skills/{*path}` — remove one learnt skill.
//!
//! Two things shape the shapes below:
//!
//! 1. **A skill's identity is its path without `.md`**, relative to
//!    [`crate::mind::skills::skills_dir`] — that is what the tree is addressed by
//!    elsewhere (`see skills/feishu-watch.md`), and the tree nests, so every route here
//!    takes a wildcard path and must guard it against traversal.
//! 2. **`_builtin/` is not user data.** [`crate::mind::skills::install_builtin_skills`]
//!    rewrites that subtree on every boot, so deleting a file in it removes nothing
//!    durable — the next start puts it back. `DELETE` refuses it outright and says why,
//!    rather than reporting a success the next boot undoes.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::foundation::server::AppState;
use crate::mind::skills::skills_dir;

/// How much prose the list carries per skill. Enough for the view to show what a note
/// is about without opening it; short enough that the whole workshop stays one small
/// response.
const EXCERPT_CHARS: usize = 160;

/// The factory-seed subtree. Rewritten on every boot, hence never deletable here.
const BUILTIN: &str = "_builtin";

// ── path safety ───────────────────────────────────────────────────────────────

/// The traversal guard for every wildcard route in this module: reject an empty path
/// and any segment that is empty, `.` or `..`, so the joined path cannot climb out of
/// the workshop root. Same rule as [`crate::foundation::server::generated`] uses for
/// the views tree — an absolute path fails it too, because a leading `/` produces an
/// empty first segment.
fn safe_rel_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\0')
        && path
            .split('/')
            .all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

/// Whether a skill path belongs to the factory layer (`_builtin` itself or anything
/// under it).
fn is_builtin(rel: &str) -> bool {
    rel == BUILTIN || rel.starts_with(&format!("{BUILTIN}/"))
}

/// Normalise the `{*path}` segment to a skill identity: the relative path *without*
/// `.md`. Callers may address a skill either way; the tree stores one file.
fn skill_id(path: &str) -> &str {
    path.strip_suffix(".md").unwrap_or(path)
}

/// The on-disk file for a skill identity.
fn skill_file(data_dir: &std::path::Path, rel: &str) -> PathBuf {
    skills_dir(data_dir).join(format!("{rel}.md"))
}

// ── markdown reading ──────────────────────────────────────────────────────────

/// Drop a leading `---` frontmatter block, if any, so the title and excerpt come from
/// the note's own words rather than its metadata.
fn strip_frontmatter(text: &str) -> &str {
    if !text.starts_with("---\n") {
        return text;
    }
    let body = &text[4..];
    let mut off = 0usize;
    for line in body.split_inclusive('\n') {
        off += line.len();
        if line.trim_end() == "---" {
            return &body[off..];
        }
    }
    text
}

/// The note's own title: a `#` heading standing *before* any prose. A heading found
/// further down is a section, not a title, so prose ends the search.
fn first_heading(body: &str) -> Option<String> {
    for line in body.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let rest = t.strip_prefix('#')?;
        let name = rest.trim_start_matches('#').trim();
        return (!name.is_empty()).then(|| name.to_string());
    }
    None
}

/// A label from the filename: `adding-a-device` → `adding a device`. The fallback when
/// the note carries no title heading.
fn label_from_stem(stem: &str) -> String {
    stem.replace(['-', '_'], " ").trim().to_string()
}

/// The first ~[`EXCERPT_CHARS`] of prose: headings, blank lines and fenced code are
/// skipped, the remaining lines joined with spaces. A note that is nothing but a
/// heading falls back to that heading's text with the `#` markers stripped, so the
/// excerpt is never empty when the file has any content at all.
fn excerpt(body: &str) -> String {
    let mut prose = String::new();
    let mut heading_fallback: Option<String> = None;
    let mut fenced = false;
    for line in body.lines() {
        let t = line.trim();
        if t.starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced || t.is_empty() {
            continue;
        }
        if let Some(rest) = t.strip_prefix('#') {
            if heading_fallback.is_none() {
                let h = rest.trim_start_matches('#').trim();
                if !h.is_empty() {
                    heading_fallback = Some(h.to_string());
                }
            }
            continue;
        }
        if !prose.is_empty() {
            prose.push(' ');
        }
        prose.push_str(t);
        if prose.chars().count() >= EXCERPT_CHARS {
            break;
        }
    }
    if prose.is_empty() {
        prose = heading_fallback.unwrap_or_default();
    }
    truncate_chars(&prose, EXCERPT_CHARS)
}

/// Cut to `max` characters (not bytes — skills are written in whatever language the
/// job was done in), marking the cut so the view doesn't have to guess.
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    format!("{}…", head.trim_end())
}

/// A file's mtime as `2026-08-04T10:00:00Z`. Falls back to the epoch when the platform
/// has no mtime — a missing timestamp must not drop the skill from the list.
fn rfc3339(t: SystemTime) -> String {
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    chrono::DateTime::from_timestamp(secs, 0)
        .unwrap_or_else(|| chrono::DateTime::from_timestamp(0, 0).expect("unix epoch is valid"))
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

// ── list ──────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct SkillDto {
    /// The skill's identity: path under `skills/`, no `.md`.
    path: String,
    /// Human label — the note's title heading, else the filename.
    name: String,
    /// True for the factory layer, which is reseeded every boot and not deletable.
    builtin: bool,
    bytes: u64,
    modified: String,
    excerpt: String,
}

/// Walk the workshop and read every `.md` in it. Iterative (a stack of dirs) rather
/// than recursive so no boxing is needed; dotfiles are skipped as editor/OS litter.
/// A missing root is an empty workshop, not an error — nothing has been learnt yet.
async fn walk_skills(root: &std::path::Path) -> std::io::Result<Vec<SkillDto>> {
    let mut found: Vec<(SystemTime, SkillDto)> = Vec::new();
    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, prefix)) = stack.pop() {
        let mut rd = match tokio::fs::read_dir(&dir).await {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        while let Some(ent) = rd.next_entry().await? {
            let name = ent.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let rel = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            // An entry we cannot stat (a broken symlink, a file removed mid-walk) is
            // skipped rather than failing the whole listing.
            let Ok(ft) = ent.file_type().await else { continue };
            if ft.is_dir() {
                stack.push((ent.path(), rel));
                continue;
            }
            let Some(stem) = name.strip_suffix(".md") else {
                continue;
            };
            let Ok(meta) = ent.metadata().await else { continue };
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let text = tokio::fs::read_to_string(ent.path()).await.unwrap_or_default();
            let body = strip_frontmatter(&text);
            let id = rel.strip_suffix(".md").unwrap_or(&rel).to_string();
            found.push((
                modified,
                SkillDto {
                    builtin: is_builtin(&id),
                    path: id,
                    name: first_heading(body).unwrap_or_else(|| label_from_stem(stem)),
                    bytes: meta.len(),
                    modified: rfc3339(modified),
                    excerpt: excerpt(body),
                },
            ));
        }
    }
    // Learnt skills first, freshest first: the note the agent just wrote is the one
    // worth looking at, and the factory layer is the same on every install.
    found.sort_by(|a, b| {
        a.1.builtin
            .cmp(&b.1.builtin)
            .then(b.0.cmp(&a.0))
            .then(a.1.path.cmp(&b.1.path))
    });
    Ok(found.into_iter().map(|(_, dto)| dto).collect())
}

/// `GET /api/skills` — every skill in the workshop, learnt ones first.
pub async fn get_skills(State(state): State<Arc<AppState>>) -> Response {
    match walk_skills(&skills_dir(&state.data_dir)).await {
        Ok(skills) => Json(serde_json::json!({ "skills": skills })).into_response(),
        Err(e) => err(&e.to_string()),
    }
}

// ── read one ──────────────────────────────────────────────────────────────────

/// `GET /api/skills/{*path}` — one skill's raw markdown, verbatim. The view renders
/// the note as written; nothing here reformats it.
pub async fn get_skill(State(state): State<Arc<AppState>>, Path(path): Path<String>) -> Response {
    if !safe_rel_path(&path) {
        return (StatusCode::NOT_FOUND, "not found\n").into_response();
    }
    let rel = skill_id(&path).to_string();
    let file = skill_file(&state.data_dir, &rel);
    let Ok(text) = tokio::fs::read_to_string(&file).await else {
        return (StatusCode::NOT_FOUND, "no such skill\n").into_response();
    };
    let modified = tokio::fs::metadata(&file)
        .await
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    Json(serde_json::json!({
        "path": rel,
        "content": text,
        "builtin": is_builtin(&rel),
        "bytes": text.len() as u64,
        "modified": rfc3339(modified),
    }))
    .into_response()
}

// ── delete one ────────────────────────────────────────────────────────────────

/// `DELETE /api/skills/{*path}` — drop a learnt skill. This is the correction verb:
/// a stale note is followed as written by every later run of that job, so removing it
/// is the fix.
///
/// Refuses anything under `_builtin/`: that layer is rewritten on the next boot, so
/// deleting it would report a success the process itself undoes.
pub async fn delete_skill(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
) -> Response {
    if !safe_rel_path(&path) {
        return err("bad skill path");
    }
    let rel = skill_id(&path).to_string();
    if is_builtin(&rel) {
        return err(
            "this is a built-in skill: the _builtin/ layer is factory seed and is rewritten on \
             every boot, so deleting it changes nothing — the next start puts it back. Only \
             skills the agent learnt can be removed.",
        );
    }
    match tokio::fs::remove_file(skill_file(&state.data_dir, &rel)).await {
        Ok(()) => Json(serde_json::json!({ "ok": true })).into_response(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => err("no such skill"),
        Err(e) => err(&e.to_string()),
    }
}

/// A uniform JSON error body with a 400.
fn err(msg: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": msg }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write `<root>/<rel>` (creating parents) — the agent writing a note.
    fn write(root: &std::path::Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn rel_path_blocks_traversal() {
        assert!(safe_rel_path("posting-a-clip"));
        assert!(safe_rel_path("video/trimming"));
        assert!(!safe_rel_path("../prompts/core"), "no parent traversal");
        assert!(!safe_rel_path("a/../b"), "no mid-path traversal");
        assert!(!safe_rel_path("a/./b"), "no dot segment");
        assert!(!safe_rel_path("/etc/passwd"), "absolute rejected via empty first segment");
        assert!(!safe_rel_path("a//b"), "no empty segment");
        assert!(!safe_rel_path(""), "empty");
    }

    #[test]
    fn builtin_is_recognised_by_prefix_only() {
        assert!(is_builtin("_builtin"));
        assert!(is_builtin("_builtin/adding-a-device"));
        assert!(!is_builtin("_builtin-notes/mine"), "a sibling name is not the layer");
        assert!(!is_builtin("video/_builtin"));
    }

    #[test]
    fn name_and_excerpt_come_from_the_note() {
        let text = "---\ntags: x\n---\n# Adding a device\n\nSSH gets you a shell.\n";
        let body = strip_frontmatter(text);
        assert_eq!(first_heading(body).unwrap(), "Adding a device");
        assert_eq!(excerpt(body), "SSH gets you a shell.");
        // Prose before a heading means the heading is a section, not a title.
        assert!(first_heading("intro line\n\n## Steps\n").is_none());
        assert_eq!(label_from_stem("adding-a-device"), "adding a device");
        assert_eq!(label_from_stem("group_watch"), "group watch");
        // Headings-only note still yields an excerpt.
        assert_eq!(excerpt("# Just a title\n"), "Just a title");
        // Fenced code is not prose.
        assert_eq!(excerpt("```\nffmpeg -ss 0\n```\nafter\n"), "after");
    }

    #[test]
    fn excerpt_is_capped_at_char_boundaries() {
        let long = "字".repeat(400);
        let out = excerpt(&long);
        assert_eq!(out.chars().count(), EXCERPT_CHARS + 1, "capped plus the marker");
        assert!(out.ends_with('…'));
    }

    #[tokio::test]
    async fn walk_lists_md_only_learnt_first() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "_builtin/adding-a-device.md", "# adding a device\n\nseeded prose.\n");
        write(root, "posting-a-clip.md", "what worked last time\n");
        write(root, "video/trimming.md", "# Trimming\n\nffmpeg -ss ...\n");
        write(root, "notes.txt", "not a skill");
        write(root, ".DS_Store", "litter");

        let skills = walk_skills(root).await.unwrap();
        let paths: Vec<&str> = skills.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(paths.len(), 3, "only .md, dotfiles skipped: {paths:?}");
        assert_eq!(*paths.last().unwrap(), "_builtin/adding-a-device", "builtin sorts last");
        assert!(paths.contains(&"video/trimming"), "nested skills are found: {paths:?}");

        let trimming = skills.iter().find(|s| s.path == "video/trimming").unwrap();
        assert_eq!(trimming.name, "Trimming", "heading beats filename");
        assert_eq!(trimming.excerpt, "ffmpeg -ss ...");
        assert!(!trimming.builtin);
        assert!(trimming.bytes > 0);
        assert!(trimming.modified.ends_with('Z'), "{}", trimming.modified);

        let clip = skills.iter().find(|s| s.path == "posting-a-clip").unwrap();
        assert_eq!(clip.name, "posting a clip", "no heading → filename label");

        assert!(skills.iter().find(|s| s.path.starts_with("_builtin/")).unwrap().builtin);
    }

    #[tokio::test]
    async fn walk_of_a_missing_workshop_is_empty_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let skills = walk_skills(&dir.path().join("skills")).await.unwrap();
        assert!(skills.is_empty());
    }

    #[test]
    fn skill_id_accepts_either_spelling() {
        assert_eq!(skill_id("video/trimming"), "video/trimming");
        assert_eq!(skill_id("video/trimming.md"), "video/trimming");
    }
}
