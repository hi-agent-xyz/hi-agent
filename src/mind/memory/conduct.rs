//! `## Working with them` — the slice of a person's facet that is read *before
//! acting*, projected into Reaction's window.
//!
//! **No new store.** What the agent has come to understand about a person already
//! lives in one place — `facets/people/<subject>/facet.md`, prose regenerated from
//! episodes with every claim citing the episodes it came from. A second file holding
//! "how to work with them" would be a second copy of the same understanding, and the
//! two would disagree within a week.
//!
//! So this is a **section convention, not a format**: one fixed heading inside that
//! prose, everything above it recall, everything under it read before acting. The
//! heading is a key code slices on; the body is the same free prose as the rest of
//! the file, in whatever language the understanding was written in.
//!
//! ## Why it has to be sliced rather than pointed at
//!
//! Reaction is tools-off, so a path to a facet is a path nobody can follow — the same
//! argument [`super::proactivity`] makes for itself. A preference the person stated
//! plainly, that the agent agreed to inside the minute, and that then failed to
//! survive the turn, is not a memory bug: it is a *projection* bug, and this is the
//! half that fixes it.
//!
//! ## Why a heading and not frontmatter
//!
//! A facet is written by judgment and rewritten whole (`docs/arch/data.md`). Imposing
//! a schema on it would make the one store in this system that is deliberately prose
//! into a form to fill in, and forms get filled. A heading costs the writer one line
//! and costs a reader nothing.
//!
//! If the heading is missing or renamed, this finds nothing and the window goes
//! without it — degraded, never wrong, which is how every other source in
//! [`super::snapshot`] fails.

use std::path::Path;

use super::{facets, layout};

/// The exact heading the section is found under. **A fixed key, not a label**: the
/// worker that writes the facet is told to use this string verbatim, and this is the
/// only thing code matches on. Kept in English regardless of the language the body is
/// written in, for the same reason a JSON key is not translated.
pub const HEADING: &str = "## Working with them";

/// Hard cap, in characters, on everything this contributes to one window.
///
/// **Three thousand, and it is code's**, like every other bound in the projection: the
/// agent decides what it has understood, not how much of Reaction's window that
/// costs. Read against the ~6k the conversation's brief may take and the ~2.5k the
/// ledger takes, it is the smallest of the three — correct, because this is the
/// slowest-moving of them. Over it, the text says so, because a ceiling that shows up
/// as text is real and one that shows up as latency is not.
pub const CONDUCT_CHARS: usize = 3_000;

/// The conduct sections of everyone the agent models, oldest-quietest first — as
/// `(subject, body)` pairs with the heading already stripped.
///
/// Only the `people` dimension is read: a project or a topic has no manner to work
/// with. Subjects with no `facet.md` are clusters the mind has not modelled yet
/// (`facets::facet_subject_index` draws the same line), and subjects whose prose
/// carries no section contribute nothing — both are ordinary, and both are silent.
pub async fn read(data_dir: &Path) -> Vec<(String, String)> {
    let people = layout::facets_dir(data_dir).join("people");
    let mut dir = match tokio::fs::read_dir(&people).await {
        Ok(rd) => rd,
        Err(_) => return Vec::new(),
    };

    let mut out: Vec<(String, String)> = Vec::new();
    while let Ok(Some(ent)) = dir.next_entry().await {
        if !matches!(ent.file_type().await.map(|t| t.is_dir()), Ok(true)) {
            continue;
        }
        let Ok(subject) = ent.file_name().into_string() else {
            continue;
        };
        if subject.is_empty() || subject.starts_with('.') {
            continue;
        }
        let path = ent.path().join(facets::FACET_FILE);
        let Ok(body) = tokio::fs::read_to_string(&path).await else {
            continue;
        };
        if let Some(section) = section(&body) {
            out.push((subject, section.to_owned()));
        }
    }
    // Stable across turns: a window that reorders itself between two turns of the same
    // conversation reads as new information when nothing changed.
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The body under [`HEADING`], or `None` when the prose does not carry one.
///
/// Runs to the next heading at any level — a facet's later `##` sections are its own
/// business and must not be swept in — or to the end of the file.
pub fn section(markdown: &str) -> Option<&str> {
    let start = markdown.lines().position(|l| l.trim_end() == HEADING)?;

    let mut begin = None;
    let mut end = markdown.len();
    let mut offset = 0usize;
    for (i, line) in markdown.lines().enumerate() {
        let line_start = offset;
        offset += line.len() + 1; // `lines()` drops one separator; close enough for a slice
        if i <= start {
            if i == start {
                begin = Some(offset.min(markdown.len()));
            }
            continue;
        }
        if line.trim_start().starts_with('#') {
            end = line_start;
            break;
        }
    }
    let body = markdown.get(begin?..end.max(begin?))?.trim();
    (!body.is_empty()).then_some(body)
}

/// Everything Reaction must know about how to be with the people in front of it,
/// as one block — or `""` when nobody has been understood that far yet, which is
/// the ordinary state of a fresh install.
///
/// Bounded by [`CONDUCT_CHARS`] across all subjects together, and the cut is
/// announced in the injected text, addressed to the one who can act on it.
/// Drop the `[[memory-link]]` citations a facet carries.
///
/// **Reaction cannot follow one.** It is tools-off by design — no file access, nothing to
/// open a link with — so a citation reaching it is provenance addressed to a reader that
/// isn't there. Cognition and Reflection read the facet itself and keep every one of them.
///
/// It is not free to leave in. Measured on one live window: 19 citations, 1,014 characters,
/// **31% of a section already over the cap** — so the host was truncating real guidance
/// about a real person to make room for slugs nobody in that seat can use.
fn without_citations(section: &str) -> String {
    let mut out = String::with_capacity(section.len());
    let mut rest = section;
    while let Some(open) = rest.find("[[") {
        let Some(close) = rest[open..].find("]]") else { break };
        out.push_str(&rest[..open]);
        // The space that introduced the citation goes with it, or a sentence ends on a gap.
        while out.ends_with(' ') {
            out.pop();
        }
        rest = &rest[open + close + 2..];
    }
    out.push_str(rest);
    // A line that held nothing but citations is now an empty line in the middle of a
    // paragraph, which reads as a break the author did not write.
    while out.contains("\n\n\n") {
        out = out.replace("\n\n\n", "\n\n");
    }
    out
}

pub async fn projection(data_dir: &Path) -> String {
    use std::fmt::Write as _;

    let people = read(data_dir).await;
    if people.is_empty() {
        return String::new();
    }

    let mut body = String::new();
    for (subject, section) in &people {
        let _ = writeln!(body, "**{subject}** — {}\n", without_citations(section));
    }
    let body = body.trim();

    let mut s = String::from("## Working with them\n");
    if body.chars().count() <= CONDUCT_CHARS {
        s.push_str(body);
        s.push('\n');
        return s;
    }
    s.extend(body.chars().take(CONDUCT_CHARS));
    let _ = write!(
        s,
        "\n\n[Cut here by the host: what you have understood about working with people \
runs past the {CONDUCT_CHARS}-character cap, so the rest is missing from this window. \
Keep each person's section to what actually changes what you do — whatever doesn't \
fit, you go without.]\n"
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_reads_to_the_next_heading() {
        let md = "# 赵力\n\nHe runs the company.\n\n## Working with them\n\nReply in Chinese.\nDone means pushed.\n\n## Something else\n\nNot this.\n";
        assert_eq!(section(md).unwrap(), "Reply in Chinese.\nDone means pushed.");
    }

    #[test]
    fn section_reads_to_end_of_file() {
        let md = "# 赵力\n\n## Working with them\n\nJust do it on his own repos.\n";
        assert_eq!(section(md).unwrap(), "Just do it on his own repos.");
    }

    #[test]
    fn a_facet_without_the_heading_contributes_nothing() {
        let md = "# 糯米\n\nThe kid. Likes pandas.\n\n## Voice\n\nHigh.\n";
        assert!(section(md).is_none());
    }

    #[test]
    fn an_empty_section_is_not_a_section() {
        let md = "# 赵力\n\n## Working with them\n\n\n## Next\n\nx\n";
        assert!(section(md).is_none());
    }

    #[test]
    fn a_deeper_heading_still_ends_it() {
        // `###` under the section would read as part of it to a human, but the writer is
        // told to keep this flat; ending here is the safe direction — it under-collects
        // rather than sweeping the rest of the file into the window.
        let md = "## Working with them\n\nKeep it short.\n\n### Aside\n\nNot this.\n";
        assert_eq!(section(md).unwrap(), "Keep it short.");
    }

    #[tokio::test]
    async fn projection_gathers_people_and_skips_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let root = layout::facets_dir(dir.path());

        for (dim, subject, body) in [
            ("people", "zhaoli", "# 赵力\n\n## Working with them\n\nChinese by default.\n"),
            ("people", "nuomi", "# 糯米\n\nThe kid.\n"),
            ("projects", "ktv", "# KTV\n\n## Working with them\n\nnot a person\n"),
        ] {
            let d = root.join(dim).join(subject);
            tokio::fs::create_dir_all(&d).await.unwrap();
            tokio::fs::write(d.join(facets::FACET_FILE), body).await.unwrap();
        }

        let out = projection(dir.path()).await;
        assert!(out.starts_with("## Working with them\n"));
        assert!(out.contains("**zhaoli** — Chinese by default."));
        // A person with no section, and a non-person dimension, are both silent.
        assert!(!out.contains("nuomi"));
        assert!(!out.contains("not a person"));
    }

    /// The whole seam, end to end, on prose shaped the way a facet actually reads —
    /// biography above the heading, conduct under it, refs on every claim. What this
    /// pins is that Reaction receives *only* the second half: the org chart and the
    /// product history are recall, and paying for them on every turn is what a bounded
    /// window cannot afford.
    #[tokio::test]
    async fn the_voice_gets_the_manner_and_not_the_biography() {
        let dir = tempfile::tempdir().unwrap();
        let d = layout::facets_dir(dir.path()).join("people").join("zhaoli");
        tokio::fs::create_dir_all(&d).await.unwrap();
        tokio::fs::write(
            d.join(facets::FACET_FILE),
            "\
# 赵力

Runs 小圆猪科技 and is the authorization point for the Feishu IT automation. His boss
is 芳姐. [[2026-08-11-nas-account-opened]]

## Working with them

Done means committed, pushed, deployed and checked — a doc update finished is not the
work finished. He has said this three separate ways since 08-10, twice while annoyed.
[[2026-08-10-doubao-ref-marked-done-early]] [[2026-08-12-only-pushed-is-done]]

On his own machines and repos, just do it — no PR, no running commentary. Four calm
instructions over ten weeks and dozens of clean runs behind it; one bad afternoon is
not a reason to start asking. [[2026-07-07-commit-straight-to-main]]

## His product vision

Wants a kid-playable, do-anything assistant. Not relevant mid-turn.
",
        )
        .await
        .unwrap();

        let out = projection(dir.path()).await;
        assert_eq!(
            out,
            "\
## Working with them
**zhaoli** — Done means committed, pushed, deployed and checked — a doc update finished is not the
work finished. He has said this three separate ways since 08-10, twice while annoyed.

On his own machines and repos, just do it — no PR, no running commentary. Four calm
instructions over ten weeks and dozens of clean runs behind it; one bad afternoon is
not a reason to start asking.
"
        );
        assert!(!out.contains("芳姐"), "biography is recall, not projection");
        assert!(!out.contains("kid-playable"), "later sections stay out");
    }

    #[tokio::test]
    async fn nothing_understood_yet_is_an_empty_block() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(projection(dir.path()).await, "");
    }

    #[tokio::test]
    async fn over_the_cap_the_cut_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let d = layout::facets_dir(dir.path()).join("people").join("zhaoli");
        tokio::fs::create_dir_all(&d).await.unwrap();
        let long = "x".repeat(CONDUCT_CHARS + 500);
        tokio::fs::write(d.join(facets::FACET_FILE), format!("{HEADING}\n\n{long}\n"))
            .await
            .unwrap();

        let out = projection(dir.path()).await;
        assert!(out.contains("Cut here by the host"));
        assert!(out.chars().count() < CONDUCT_CHARS + 500);
    }
}

#[cfg(test)]
mod citation_tests {
    use super::*;

    /// Reaction cannot open a `[[link]]`, and on one live window 19 of them took 1,014
    /// characters — 31% of a section the host was already truncating. The guidance stays;
    /// the addresses go.
    #[test]
    fn citations_go_and_the_sentence_survives() {
        let with = "Done means deployed. [[2026-08-10-marked-done-early]] [[2026-08-12-only-pushed]]";
        assert_eq!(without_citations(with), "Done means deployed.");
    }

    /// A line that was nothing but citations must not leave a paragraph break behind it.
    #[test]
    fn a_line_of_only_citations_leaves_no_hole() {
        let with = "First thing.\n[[a-note]] [[b-note]]\n\nSecond thing.";
        assert_eq!(without_citations(with), "First thing.\n\nSecond thing.");
    }

    /// An unclosed `[[` is text, not the start of a citation — the section is prose written
    /// by an agent, and it must never lose the rest of itself to a stray bracket.
    #[test]
    fn an_unclosed_bracket_keeps_everything_after_it() {
        let odd = "He writes [[ like this sometimes, and the rest matters.";
        assert_eq!(without_citations(odd), odd);
    }
}
