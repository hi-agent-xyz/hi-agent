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
//! 2. **`factory/` is not user data.** [`crate::mind::skills::install_factory_skills`]
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
const FACTORY: &str = "factory";

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

/// Whether a skill path belongs to the factory layer (`factory` itself or anything
/// under it).
fn is_factory(rel: &str) -> bool {
    rel == FACTORY || rel.starts_with(&format!("{FACTORY}/"))
}

/// Normalise the `{*path}` segment to a skill identity: the relative path *without*
/// `.md`. Callers may address a skill either way; the tree stores one file.
fn skill_id(path: &str) -> &str {
    path.strip_suffix(".md").unwrap_or(path)
}

/// The on-disk file for a skill identity — either shape.
///
/// A note is `<id>.md`, **or** `<id>/SKILL.md` for the directory shape the agent
/// runtime's own skills feature writes. The flat file is preferred when both exist,
/// matching [`crate::mind::skills::split_front_matter`]'s preference for our own
/// spelling; a caller that addressed `<id>` gets whichever is actually there.
fn skill_file(data_dir: &std::path::Path, rel: &str) -> PathBuf {
    let flat = skills_dir(data_dir).join(format!("{rel}.md"));
    if flat.is_file() {
        return flat;
    }
    let nested = skills_dir(data_dir).join(rel).join(crate::mind::skills::SKILL_FILE);
    if nested.is_file() { nested } else { flat }
}

// ── markdown reading ──────────────────────────────────────────────────────────

// Front matter is read by [`crate::mind::skills::split_front_matter`], which owns the
// vocabulary: `purpose` is what the registry scan emits and the presence of `use` is
// what makes a note a tool. A second parser here would be free to disagree with it
// about what a note *is*, so there isn't one.

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
    /// The command this note names, when it names one — `use:` in the front matter.
    /// Present iff the note is a **tool** rather than an ordinary procedure, which is
    /// the only distinction the reader needs: a tool can be run, a skill is followed.
    #[serde(skip_serializing_if = "Option::is_none")]
    run: Option<String>,
}

/// Walk the workshop and read every note in it. Iterative (a stack of dirs) rather
/// than recursive so no boxing is needed; dotfiles are skipped as editor/OS litter.
/// A missing root is an empty workshop, not an error — nothing has been learnt yet.
///
/// Two passes: the walk decides *which files are notes* (a `.md`, or a directory's
/// `SKILL.md`, which also ends the descent), then each note is read. Splitting them
/// keeps the "is this a note" rule in one place instead of interleaved with parsing.
async fn walk_skills(root: &std::path::Path) -> std::io::Result<Vec<SkillDto>> {
    let mut found: Vec<(SystemTime, SkillDto)> = Vec::new();
    let mut notes: Vec<(PathBuf, String)> = Vec::new();
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
                // **A directory holding a `SKILL.md` is one note, not a subtree to
                // walk.** Everything beside that file is the tool's payload — a
                // vendored dependency tree, a script, its fixtures — and walking in
                // listed a bundled `LICENSE.md` *as a skill* on the run that first
                // produced this shape. The note is read here; the payload is not
                // knowledge and is nobody's reading material.
                if ent.path().join(crate::mind::skills::SKILL_FILE).is_file() {
                    notes.push((ent.path().join(crate::mind::skills::SKILL_FILE), rel));
                } else {
                    stack.push((ent.path(), rel));
                }
                continue;
            }
            if name.ends_with(".md") {
                notes.push((ent.path(), rel));
            }
        }
    }

    for (path, rel) in notes {
        // The identity is the path without `.md` — or, for the directory shape, the
        // directory itself: `extract-webpage-markdown/SKILL.md` is the tool
        // `extract-webpage-markdown`. Either way the name comes from the tree.
        {
            let id = rel.strip_suffix(".md").unwrap_or(&rel).to_string();
            let stem = id.rsplit('/').next().unwrap_or(&id).to_string();
            let stem = stem.as_str();
            let Ok(meta) = tokio::fs::metadata(&path).await else { continue };
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let text = tokio::fs::read_to_string(&path).await.unwrap_or_default();
            let (fm, body) = crate::mind::skills::split_front_matter(&text);
            found.push((
                modified,
                SkillDto {
                    builtin: is_factory(&id),
                    path: id,
                    name: first_heading(body).unwrap_or_else(|| label_from_stem(stem)),
                    bytes: meta.len(),
                    modified: rfc3339(modified),
                    // `purpose` *is* the one-line summary, written to be matched
                    // against a job — so when a note has one it beats anything
                    // scraped from the prose, which for a tool note is whatever
                    // sentence happened to come first.
                    excerpt: fm.purpose.clone().unwrap_or_else(|| excerpt(body)),
                    run: fm.run,
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
        "builtin": is_factory(&rel),
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
/// Refuses anything under `factory/`: that layer is rewritten on the next boot, so
/// deleting it would report a success the process itself undoes.
pub async fn delete_skill(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
) -> Response {
    if !safe_rel_path(&path) {
        return err("bad skill path");
    }
    let rel = skill_id(&path).to_string();
    if is_factory(&rel) {
        return err(
            "this is a built-in skill: the factory/ layer is factory seed and is rewritten on \
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
        assert!(is_factory("factory"));
        assert!(is_factory("factory/adding-a-device"));
        assert!(!is_factory("factory-notes/mine"), "a sibling name is not the layer");
        assert!(!is_factory("video/factory"));
    }

    /// **A skill directory is one note, and its payload is not knowledge.** Watched
    /// 2026-08-27: a learnt tool arrived as `extract-webpage-markdown/SKILL.md` with
    /// 76 MB of vendored Python beside it, and the walk listed a bundled
    /// `LICENSE.md` *as a skill*. So `SKILL.md` ends the descent, and the identity is
    /// the directory — the name still comes from the tree, one level up.
    #[tokio::test]
    async fn a_skill_directory_is_one_note_and_its_payload_is_not_walked() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path();
        let root = &skills_dir(data_dir);
        let tool = root.join("extract-webpage-markdown");
        std::fs::create_dir_all(tool.join("scripts/vendor/soupsieve/licenses")).unwrap();
        std::fs::write(
            tool.join(crate::mind::skills::SKILL_FILE),
            "---\ndescription: Extract a page into Markdown\nuse: extract-md\n---\n\n# Extract\n\nbody\n",
        )
        .unwrap();
        std::fs::write(tool.join("scripts/vendor/soupsieve/licenses/LICENSE.md"), "MIT License\n").unwrap();
        // An ordinary flat note, and a real subtree that must still be walked.
        std::fs::create_dir_all(root.join("video")).unwrap();
        std::fs::write(root.join("video/trimming.md"), "# Trimming\n\nffmpeg -ss 0\n").unwrap();

        let listed = walk_skills(root).await.unwrap();
        let paths: Vec<&str> = listed.iter().map(|s| s.path.as_str()).collect();
        assert!(paths.contains(&"extract-webpage-markdown"), "got {paths:?}");
        assert!(paths.contains(&"video/trimming"), "a real subtree is still walked: {paths:?}");
        assert!(
            !paths.iter().any(|p| p.contains("vendor")),
            "the payload is not reading material: {paths:?}"
        );

        let tool_dto = listed.iter().find(|s| s.path == "extract-webpage-markdown").unwrap();
        assert_eq!(tool_dto.run.as_deref(), Some("extract-md"));
        assert_eq!(tool_dto.excerpt, "Extract a page into Markdown");

        // The identity the listing reports must round-trip: `GET` and `DELETE` resolve
        // it back to the file on disk, in either shape. A listing that names something
        // the other verbs cannot open is the bug this guards.
        assert_eq!(
            skill_file(data_dir, "extract-webpage-markdown"),
            tool.join(crate::mind::skills::SKILL_FILE)
        );
        assert_eq!(skill_file(data_dir, "video/trimming"), root.join("video/trimming.md"));
    }

    #[test]
    fn name_and_excerpt_come_from_the_note() {
        let text = "---\ntags: x\n---\n# Adding a device\n\nSSH gets you a shell.\n";
        let (_, body) = crate::mind::skills::split_front_matter(text);
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
        write(root, "factory/adding-a-device.md", "# adding a device\n\nseeded prose.\n");
        write(root, "posting-a-clip.md", "what worked last time\n");
        write(root, "video/trimming.md", "# Trimming\n\nffmpeg -ss ...\n");
        write(root, "notes.txt", "not a skill");
        write(root, ".DS_Store", "litter");

        let skills = walk_skills(root).await.unwrap();
        let paths: Vec<&str> = skills.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(paths.len(), 3, "only .md, dotfiles skipped: {paths:?}");
        assert_eq!(*paths.last().unwrap(), "factory/adding-a-device", "builtin sorts last");
        assert!(paths.contains(&"video/trimming"), "nested skills are found: {paths:?}");

        let trimming = skills.iter().find(|s| s.path == "video/trimming").unwrap();
        assert_eq!(trimming.name, "Trimming", "heading beats filename");
        assert_eq!(trimming.excerpt, "ffmpeg -ss ...");
        assert!(!trimming.builtin);
        assert!(trimming.bytes > 0);
        assert!(trimming.modified.ends_with('Z'), "{}", trimming.modified);

        let clip = skills.iter().find(|s| s.path == "posting-a-clip").unwrap();
        assert_eq!(clip.name, "posting a clip", "no heading → filename label");

        assert!(skills.iter().find(|s| s.path.starts_with("factory/")).unwrap().builtin);
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
