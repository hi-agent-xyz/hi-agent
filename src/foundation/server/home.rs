//! `factory/home`'s own state: the grouping the work in hand is arranged into.
//!
//! A group is a name, an ordered list of task subjects and the groups inside it, and it
//! belongs to this one surface (`docs/arch/home.md#grouping`). Deliberately **not** a field on the task record:
//! that would commit every reader of the ledger to one axis — project, or kind, or state —
//! for the sake of one view, and the axis is the person's to change.
//!
//! **Code never infers a group.** The rank that once did was deleted for being inferred:
//! `systems` names the operational records a task touches, so a birthday deck whose photos
//! arrived over Feishu was drawn under "feishu" beside a client brief. Evidence like that is
//! an input to whoever *writes* the grouping, never a rule here.
//!
//! Two files, because they have two readers:
//!
//! - `<data_dir>/home/grouping.md` — the person's standing instructions in their own words.
//!   Read by a mind when it groups, **never parsed here**, so nothing it can contain breaks
//!   the surface. Written like any other file.
//! - `<data_dir>/home/groups.json` — the arrangement that stands. Parsed on every poll and
//!   rendered, so it is JSON and it is written only through [`write`], which validates and
//!   renames a temp sibling into place. A surface polling every few seconds must never read
//!   half a file, and a mistyped subject must fail where the mind can still fix it rather
//!   than go silently missing minutes later.
//!
//! What is open, what is running, what each task is called: none of that is stored here. The
//! mind that groups has the active-task projection and the switchboard in front of it
//! already, and a copy of them would be a second ledger going stale.
//!
//! **A group's icon is drawn by the mind that groups, from one picture**
//! (`docs/arch/home.md#icons`). Every group wears [`DEFAULT_ICON`] until it has its own, and
//! every icon of its own is an edit of that same picture — [`ICON_ANCHOR_REF`] is where it is
//! filed for `hi_image_to_image` to take. A style described in words drifts a little with
//! every draw; a style carried by the source image is what keeps eight icons drawn weeks apart
//! one set.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::foundation::server::AppState;
use crate::mind::memory::media::{self, DRIVE_PREFIX};
use crate::mind::memory::{facets, tasks};

/// The icon a group wears until one is drawn for it, and the picture every drawn one is an
/// edit of. Bundled, so a fresh install has both before anything has been generated.
pub const DEFAULT_ICON: &[u8] = include_bytes!("assets/home-group-icon.png");

/// Where [`DEFAULT_ICON`] is filed in the drive, so a mind can hand it to `hi_image_to_image`.
pub const ICON_ANCHOR_REF: &str = "drive/home/group-icon.png";

/// Where the icon-sized copies live, under the drive. A generation comes back at 1024px or
/// more and a megabyte or two; Home draws it at 40px on every open, so the copy is what is
/// recorded and the original stays where it was made.
const ICONS_DIR: &str = "home/icons";

/// 40px at 3x.
const ICON_PX: u32 = 120;

/// `<data_dir>/home` — one surface's state, and nothing else reads it.
pub fn home_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("home")
}

/// `<data_dir>/home/groups.json` — the arrangement that currently stands.
pub fn groups_path(data_dir: &Path) -> PathBuf {
    home_dir(data_dir).join("groups.json")
}

/// `<data_dir>/home/grouping.md` — the person's standing instructions, in their words.
///
/// Prose on purpose: a model is its only reader, and regularising an ask into fields
/// compresses it before the reader sees it. Nothing in this file is parsed.
pub fn basis_path(data_dir: &Path) -> PathBuf {
    home_dir(data_dir).join("grouping.md")
}

/// The whole arrangement. Array order is draw order — groups outward from the core.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Grouping {
    #[serde(default)]
    pub groups: Vec<Group>,
}

/// One group: what it is called, why it exists, which tasks are in it, and the groups
/// inside it.
///
/// There is no id beside the `label`, because renaming a group *is* renaming it, and no
/// `updated_at`, because the file's mtime already says when it was written.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub label: String,
    /// One line saying what this grouping was based on. Optional, and **read** — Home hangs
    /// it on the label as hover text, so a person reviewing the arrangement can see why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The group's own icon, a `drive/home/icons/…` ref. Absent means the default.
    ///
    /// **It belongs to the label, not to the write.** A rewrite that leaves it out keeps the
    /// icon that label already had, because rearranging is not redrawing: the arrangement is
    /// replaced whole every pass, and one that had to carry every icon forward by hand would
    /// lose them to a forgotten field and pay a generation apiece to get them back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Task subjects, top to bottom. A member naming no drawn task is ignored by the
    /// surface: tasks close and age out while the record stands, and the record is not the
    /// ledger.
    #[serde(default)]
    pub members: Vec<String>,
    /// Groups inside this one, drawn after its own members in this order. Same shape at
    /// every depth, and no depth limit: how finely a person divides their work is theirs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<Group>,
}

/// What stands right now, or nothing at all.
///
/// **Unreadable means ungrouped, and says so in the log.** A missing file is the ordinary
/// case before anyone has grouped anything; malformed JSON is a fault, and the honest answer
/// to both is the surface as it was — every task on the core — rather than a half-applied
/// arrangement or an error card on a person's screen.
pub async fn read(data_dir: &Path) -> Grouping {
    let path = groups_path(data_dir);
    let raw = match tokio::fs::read_to_string(&path).await {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Grouping::default(),
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "home grouping unreadable; drawing none");
            return Grouping::default();
        }
    };
    match serde_json::from_str::<Grouping>(&raw) {
        Ok(grouping) => grouping,
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "home grouping is not the shape it should be; drawing none");
            Grouping::default()
        }
    }
}

/// What a write turned out to be, for the writer to read back.
///
/// The counts are the point: a mind that mistypes a subject, or leaves half the open work
/// out, learns it in the same turn instead of discovering it on a screen minutes later.
#[derive(Debug, PartialEq)]
pub struct Written {
    /// The arrangement as it landed, after normalising.
    pub grouping: Grouping,
    /// Members naming no task on disk, dropped.
    pub unknown: Vec<String>,
    /// Members named by more than one group; the first group keeps them.
    pub duplicated: Vec<String>,
    /// Open tasks in no group at all. Ordinary, not a fault — but worth knowing.
    pub ungrouped: Vec<String>,
    /// Labels that landed wearing the default icon — the ones with nothing drawn yet.
    pub iconless: Vec<String>,
    /// `label: why` for each icon offered that could not be used. The label keeps what it had.
    pub refused_icons: Vec<String>,
}

/// Replace the arrangement whole, or refuse and change nothing.
///
/// Whole-record writes for the same reason [`facets::update_facet`] takes a whole facet: a
/// dozen rows fit in one call, and patch operations over a list are a grammar to get wrong.
///
/// Normalising, in order: labels and members are trimmed; a member that names no task
/// directory is dropped; a member claimed twice stays where it was first claimed, reading a
/// group's own members before the groups inside it; a group left with no members and no
/// groups is dropped. Two groups sharing a label — at any depth — is the one hard error: the
/// label is the group's identity, so a duplicate makes the arrangement ambiguous rather than
/// merely untidy, and nothing is written.
///
/// Icons: one offered is filed as an icon-sized copy ([`file_icon`]) and recorded; one not
/// offered is the label's previous icon, if its file is still there. An offered icon that
/// cannot be used is refused on its own — the arrangement still lands, and the label keeps
/// what it had.
pub async fn write(data_dir: &Path, proposed: Grouping) -> anyhow::Result<Written> {
    let known = facets::subjects_in(data_dir, tasks::DIMENSION).await;
    let previous: std::collections::HashMap<String, String> = every_group(&read(data_dir).await.groups)
        .into_iter()
        .filter_map(|g| Some((g.label.clone(), g.icon.clone()?)))
        .collect();
    let mut pass = Normalise {
        known: known.iter().map(String::as_str).collect(),
        labels: std::collections::HashSet::new(),
        taken: std::collections::HashSet::new(),
        unknown: Vec::new(),
        duplicated: Vec::new(),
    };
    let mut groups = pass.groups(proposed.groups)?;
    let Normalise { taken, unknown, duplicated, .. } = pass;

    let mut refused_icons = Vec::new();
    let mut icons = Vec::new();
    icons_mut(&mut groups, &mut icons);
    for (label, icon) in icons {
        let offered = icon.take().map(|i| i.trim().to_owned()).filter(|i| !i.is_empty());
        let filed = match offered {
            Some(offered) => match file_icon(data_dir, &offered).await {
                Ok(filed) => Some(filed),
                Err(error) => {
                    refused_icons.push(format!("{label}: {error}"));
                    None
                }
            },
            None => None,
        };
        *icon = match filed {
            Some(filed) => Some(filed),
            None => match previous.get(label) {
                Some(kept) if media::resolve_ref(data_dir, kept).await.is_some() => Some(kept.clone()),
                _ => None,
            },
        };
    }
    let grouping = Grouping { groups };

    let dir = home_dir(data_dir);
    tokio::fs::create_dir_all(&dir).await?;
    let path = groups_path(data_dir);
    let tmp = dir.join(format!(".groups.json.tmp-{}", Uuid::now_v7().simple()));
    tokio::fs::write(&tmp, serde_json::to_vec_pretty(&grouping)?).await?;
    tokio::fs::rename(&tmp, &path).await?;

    let iconless =
        every_group(&grouping.groups).into_iter().filter(|g| g.icon.is_none()).map(|g| g.label.clone()).collect();
    Ok(Written {
        ungrouped: ungrouped_open_tasks(data_dir, &grouping, &taken).await,
        grouping,
        unknown,
        duplicated,
        iconless,
        refused_icons,
    })
}

/// File an offered icon as the copy Home draws, and return that copy's ref.
///
/// A ref already under [`ICONS_DIR`] is a copy this function made — a writer passing back
/// what `groups.json` holds — and is kept as it is. Anything else is read from the drive,
/// cropped to its centred square, scaled to [`ICON_PX`] and filed under a fresh name, so the
/// original can be edited, moved or deleted without the screen changing.
async fn file_icon(data_dir: &Path, offered: &str) -> anyhow::Result<String> {
    let Some(rel) = offered.strip_prefix(DRIVE_PREFIX) else {
        anyhow::bail!("`{offered}` is not a `drive/…` ref");
    };
    let Some(source) = media::resolve_in_drive(data_dir, rel).await else {
        anyhow::bail!("nothing is filed at `{offered}`");
    };
    if rel.starts_with(&format!("{ICONS_DIR}/")) {
        return Ok(offered.to_owned());
    }
    let bytes = tokio::fs::read(&source).await?;
    let png = tokio::task::spawn_blocking(move || icon_sized(&bytes))
        .await?
        .map_err(|error| anyhow::anyhow!("`{offered}` is not a picture that can be read: {error}"))?;
    let dir = media::drive_root(data_dir).join(ICONS_DIR);
    tokio::fs::create_dir_all(&dir).await?;
    let name = format!("{}.png", Uuid::now_v7().simple());
    let tmp = dir.join(format!(".{name}.tmp"));
    tokio::fs::write(&tmp, &png).await?;
    tokio::fs::rename(&tmp, dir.join(&name)).await?;
    Ok(format!("{DRIVE_PREFIX}{ICONS_DIR}/{name}"))
}

/// The centred square of a picture at [`ICON_PX`], as PNG. Not blown up when smaller.
fn icon_sized(bytes: &[u8]) -> image::ImageResult<Vec<u8>> {
    let img = image::load_from_memory(bytes)?;
    let side = img.width().min(img.height());
    let square = img.crop_imm((img.width() - side) / 2, (img.height() - side) / 2, side, side);
    let scaled = if side > ICON_PX {
        square.resize_exact(ICON_PX, ICON_PX, image::imageops::FilterType::Lanczos3)
    } else {
        square
    };
    let mut out = Vec::new();
    scaled.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)?;
    Ok(out)
}

/// Put [`DEFAULT_ICON`] at [`ICON_ANCHOR_REF`], so the ref a writer is told to draw from is
/// there to be read. Written when missing or when this build's picture differs, so the
/// anchor is always the one the screen is wearing.
pub async fn file_icon_anchor(data_dir: &Path) -> anyhow::Result<()> {
    let rel = ICON_ANCHOR_REF.strip_prefix(DRIVE_PREFIX).unwrap_or(ICON_ANCHOR_REF);
    let path = media::drive_root(data_dir).join(rel);
    if tokio::fs::read(&path).await.is_ok_and(|bytes| bytes == DEFAULT_ICON) {
        return Ok(());
    }
    let dir = path.parent().unwrap_or(&path).to_path_buf();
    tokio::fs::create_dir_all(&dir).await?;
    let tmp = dir.join(format!(".group-icon.png.tmp-{}", Uuid::now_v7().simple()));
    tokio::fs::write(&tmp, DEFAULT_ICON).await?;
    tokio::fs::rename(&tmp, &path).await?;
    Ok(())
}

/// One normalising walk over the whole tree, so identity and first claim are decided across
/// every depth at once rather than per level.
struct Normalise<'a> {
    known: std::collections::HashSet<&'a str>,
    labels: std::collections::HashSet<String>,
    taken: std::collections::HashSet<String>,
    unknown: Vec<String>,
    duplicated: Vec<String>,
}

impl Normalise<'_> {
    fn groups(&mut self, proposed: Vec<Group>) -> anyhow::Result<Vec<Group>> {
        let mut groups = Vec::new();
        for group in proposed {
            let label = group.label.trim().to_owned();
            if label.is_empty() {
                anyhow::bail!("a group needs a label");
            }
            if !self.labels.insert(label.clone()) {
                anyhow::bail!("two groups are both called `{label}`; a label is a group's identity");
            }
            let mut members = Vec::new();
            for member in group.members {
                let member = member.trim().to_owned();
                if member.is_empty() {
                    continue;
                }
                if !self.known.contains(member.as_str()) {
                    self.unknown.push(member);
                    continue;
                }
                if !self.taken.insert(member.clone()) {
                    self.duplicated.push(member);
                    continue;
                }
                members.push(member);
            }
            let inner = self.groups(group.groups)?;
            if members.is_empty() && inner.is_empty() {
                continue;
            }
            let note = group.note.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty());
            groups.push(Group { label, note, icon: group.icon, members, groups: inner });
        }
        Ok(groups)
    }
}

/// Every group at every depth, each before the groups inside it.
fn every_group(groups: &[Group]) -> Vec<&Group> {
    let mut out = Vec::new();
    for group in groups {
        out.push(group);
        out.extend(every_group(&group.groups));
    }
    out
}

/// Each group's label beside its icon slot, at every depth, in the same order as
/// [`every_group`] — so icons are filed and kept the same way however deep a group sits.
fn icons_mut<'a>(groups: &'a mut [Group], out: &mut Vec<(&'a str, &'a mut Option<String>)>) {
    for group in groups {
        let Group { label, icon, groups, .. } = group;
        let label: &'a String = label;
        out.push((label.as_str(), icon));
        icons_mut(groups, out);
    }
}

/// The open tasks this arrangement does not place. Closed ones are not owed a group: they
/// leave the surface on their own, 24 hours after they close.
async fn ungrouped_open_tasks(
    data_dir: &Path,
    _grouping: &Grouping,
    taken: &std::collections::HashSet<String>,
) -> Vec<String> {
    match tasks::active_tasks(data_dir).await {
        Ok(open) => {
            open.into_iter().map(|task| task.subject).filter(|s| !taken.contains(s)).collect()
        }
        Err(error) => {
            tracing::warn!(%error, "task ledger unreadable; the write cannot say what is ungrouped");
            Vec::new()
        }
    }
}

/// `GET /api/home/groups` — the arrangement, for the surface to draw.
///
/// Which tasks are *not* in it is left to the caller, which is holding the task list
/// anyway; answering it here would be a second copy of the ledger's own judgment about
/// what is open.
pub async fn get_home_groups(State(state): State<Arc<AppState>>) -> Json<Grouping> {
    Json(read(&state.data_dir).await)
}

/// `GET /api/home/group-icon` — the icon a group wears until its own is drawn.
///
/// Served from the binary rather than from the drive copy, so it is on screen before any
/// write has filed the anchor.
pub async fn get_default_group_icon() -> impl axum::response::IntoResponse {
    ([(axum::http::header::CONTENT_TYPE, "image/png")], DEFAULT_ICON)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn task(dir: &Path, subject: &str) {
        facets::update_facet(dir, tasks::DIMENSION, subject, "---\nstatus: doing\ntitle: t\n---\n\nbody\n")
            .await
            .unwrap();
    }

    fn group(label: &str, members: &[&str]) -> Group {
        Group {
            label: label.into(),
            note: None,
            icon: None,
            members: members.iter().map(|m| (*m).into()).collect(),
            groups: Vec::new(),
        }
    }

    fn holding(label: &str, members: &[&str], groups: Vec<Group>) -> Group {
        Group { groups, ..group(label, members) }
    }

    fn with_icon(mut group: Group, icon: &str) -> Group {
        group.icon = Some(icon.into());
        group
    }

    /// A picture in the drive the size a generation comes back at, and not square.
    async fn drawn(dir: &Path, rel: &str) -> String {
        let img = image::RgbImage::from_pixel(1254, 1024, image::Rgb([240, 232, 218]));
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let path = media::drive_root(dir).join(rel);
        tokio::fs::create_dir_all(path.parent().unwrap()).await.unwrap();
        tokio::fs::write(&path, png).await.unwrap();
        format!("{DRIVE_PREFIX}{rel}")
    }

    #[tokio::test]
    async fn nothing_written_yet_is_no_groups_rather_than_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read(dir.path()).await, Grouping::default());
    }

    /// The fault a surface must survive: something wrote the file, and it is not JSON.
    #[tokio::test]
    async fn an_unreadable_record_draws_no_groups_instead_of_failing() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::create_dir_all(home_dir(dir.path())).await.unwrap();
        tokio::fs::write(groups_path(dir.path()), "{ groups: [oops").await.unwrap();
        assert_eq!(read(dir.path()).await, Grouping::default());

        // The right shape's wrong type is the same answer.
        tokio::fs::write(groups_path(dir.path()), r#"{"groups": "ktv"}"#).await.unwrap();
        assert_eq!(read(dir.path()).await, Grouping::default());
    }

    #[tokio::test]
    async fn what_is_written_is_what_comes_back() {
        let dir = tempfile::tempdir().unwrap();
        task(dir.path(), "kt8-046").await;
        task(dir.path(), "cantonese-table").await;
        let written = write(
            dir.path(),
            Grouping {
                groups: vec![Group {
                    label: "  KTV  ".into(),
                    note: Some("  9/16 说是一摊事  ".into()),
                    icon: None,
                    members: vec!["kt8-046".into(), " cantonese-table ".into()],
                    groups: Vec::new(),
                }],
            },
        )
        .await
        .unwrap();
        assert_eq!(written.grouping.groups[0].label, "KTV");
        assert_eq!(written.grouping.groups[0].note.as_deref(), Some("9/16 说是一摊事"));
        assert_eq!(written.grouping.groups[0].members, ["kt8-046", "cantonese-table"]);
        assert_eq!(read(dir.path()).await, written.grouping);
    }

    /// **A subject that does not exist is dropped and reported, not written.** The whole
    /// reason this goes through a tool: a typo that silently disappeared would surface as a
    /// task missing from a group minutes later, with nothing saying why.
    #[tokio::test]
    async fn a_member_naming_no_task_is_dropped_and_named() {
        let dir = tempfile::tempdir().unwrap();
        task(dir.path(), "kt8-046").await;
        let written =
            write(dir.path(), Grouping { groups: vec![group("KTV", &["kt8-046", "kt8-999"])] })
                .await
                .unwrap();
        assert_eq!(written.grouping.groups[0].members, ["kt8-046"]);
        assert_eq!(written.unknown, ["kt8-999"]);
    }

    #[tokio::test]
    async fn a_task_claimed_twice_stays_in_the_first_group() {
        let dir = tempfile::tempdir().unwrap();
        task(dir.path(), "kt8-046").await;
        task(dir.path(), "vocabulary-book").await;
        let written = write(
            dir.path(),
            Grouping {
                groups: vec![group("KTV", &["kt8-046"]), group("家里", &["kt8-046", "vocabulary-book"])],
            },
        )
        .await
        .unwrap();
        assert_eq!(written.grouping.groups[0].members, ["kt8-046"]);
        assert_eq!(written.grouping.groups[1].members, ["vocabulary-book"]);
        assert_eq!(written.duplicated, ["kt8-046"]);
    }

    /// A group with nothing in it draws nothing, so it is not an arrangement — it is a
    /// heading somebody forgot to fill. Emptied by dropped members or written empty, same.
    #[tokio::test]
    async fn a_group_with_no_members_is_not_kept() {
        let dir = tempfile::tempdir().unwrap();
        task(dir.path(), "kt8-046").await;
        let written = write(
            dir.path(),
            Grouping { groups: vec![group("KTV", &["kt8-046"]), group("空的", &["gone"])] },
        )
        .await
        .unwrap();
        assert_eq!(written.grouping.groups.len(), 1);
        assert_eq!(written.grouping.groups[0].label, "KTV");
    }

    /// The one hard error. Everything else normalises, because everything else has an
    /// obvious right answer; two groups called the same thing has none.
    #[tokio::test]
    async fn two_groups_with_one_label_write_nothing() {
        let dir = tempfile::tempdir().unwrap();
        task(dir.path(), "kt8-046").await;
        task(dir.path(), "vocabulary-book").await;
        write(dir.path(), Grouping { groups: vec![group("KTV", &["kt8-046"])] }).await.unwrap();
        let refused = write(
            dir.path(),
            Grouping {
                groups: vec![group("同名", &["kt8-046"]), group("同名", &["vocabulary-book"])],
            },
        )
        .await;
        assert!(refused.is_err());
        // Refused means unchanged, not half-applied.
        assert_eq!(read(dir.path()).await.groups[0].label, "KTV");
    }

    /// The receipt that closes the loop: what is open and in no group.
    #[tokio::test]
    async fn the_write_says_which_open_work_it_left_out() {
        let dir = tempfile::tempdir().unwrap();
        task(dir.path(), "kt8-046").await;
        task(dir.path(), "vocabulary-book").await;
        let written =
            write(dir.path(), Grouping { groups: vec![group("KTV", &["kt8-046"])] }).await.unwrap();
        assert_eq!(written.ungrouped, ["vocabulary-book"]);
    }

    /// **A group can hold groups, and the same rules hold at every depth.** A parent may carry
    /// its own members as well as its groups, and one that carries only groups is kept.
    #[tokio::test]
    async fn groups_inside_a_group_come_back_as_written() {
        let dir = tempfile::tempdir().unwrap();
        for subject in ["rollup", "site-survey", "field-notes", "vocabulary-book"] {
            task(dir.path(), subject).await;
        }
        let arranged = Grouping {
            groups: vec![
                holding(
                    "Client work",
                    &["rollup"],
                    vec![
                        group("Site", &["site-survey"]),
                        holding("Research", &[], vec![group("Notes", &["field-notes"])]),
                    ],
                ),
                group("Home", &["vocabulary-book"]),
            ],
        };
        let written = write(dir.path(), arranged.clone()).await.unwrap();
        assert_eq!(written.grouping, arranged);
        assert!(written.ungrouped.is_empty(), "a task inside an inner group is placed");
        assert_eq!(read(dir.path()).await, arranged);
    }

    /// Identity and first claim are decided across the whole tree, not level by level: a
    /// group's own members are read before the groups inside it.
    #[tokio::test]
    async fn a_label_or_a_claim_is_unique_across_every_depth() {
        let dir = tempfile::tempdir().unwrap();
        task(dir.path(), "a").await;
        task(dir.path(), "b").await;
        let refused = write(
            dir.path(),
            Grouping { groups: vec![holding("Work", &["a"], vec![group("Work", &["b"])])] },
        )
        .await;
        assert!(refused.is_err(), "an inner group may not reuse an outer label");

        let written = write(
            dir.path(),
            Grouping { groups: vec![holding("Work", &["a"], vec![group("Inner", &["a", "b"])])] },
        )
        .await
        .unwrap();
        assert_eq!(written.grouping.groups[0].members, ["a"]);
        assert_eq!(written.grouping.groups[0].groups[0].members, ["b"]);
        assert_eq!(written.duplicated, ["a"]);
    }

    /// A parent whose inner groups all empty out is empty too, and goes with them.
    #[tokio::test]
    async fn a_group_whose_inner_groups_are_all_empty_is_not_kept() {
        let dir = tempfile::tempdir().unwrap();
        task(dir.path(), "a").await;
        let written = write(
            dir.path(),
            Grouping {
                groups: vec![group("Kept", &["a"]), holding("Hollow", &[], vec![group("Gone", &["nope"])])],
            },
        )
        .await
        .unwrap();
        assert_eq!(written.grouping.groups.len(), 1);
        assert_eq!(written.grouping.groups[0].label, "Kept");
        assert_eq!(written.unknown, ["nope"]);
    }

    /// A record written before groups could nest reads back unchanged, and is written back
    /// without an empty `groups` on every group.
    #[tokio::test]
    async fn a_flat_record_reads_and_writes_as_it_always_did() {
        let dir = tempfile::tempdir().unwrap();
        task(dir.path(), "kt8-046").await;
        tokio::fs::create_dir_all(home_dir(dir.path())).await.unwrap();
        tokio::fs::write(groups_path(dir.path()), r#"{"groups":[{"label":"KTV","members":["kt8-046"]}]}"#)
            .await
            .unwrap();
        let flat = read(dir.path()).await;
        assert_eq!(flat, Grouping { groups: vec![group("KTV", &["kt8-046"])] });
        write(dir.path(), flat).await.unwrap();
        let raw = tokio::fs::read_to_string(groups_path(dir.path())).await.unwrap();
        assert!(!raw.contains("\"groups\": []"), "{raw}");
    }

    /// Clearing is a write like any other — the person can ask for no grouping at all.
    #[tokio::test]
    async fn an_empty_arrangement_is_a_legal_one() {
        let dir = tempfile::tempdir().unwrap();
        task(dir.path(), "kt8-046").await;
        write(dir.path(), Grouping { groups: vec![group("KTV", &["kt8-046"])] }).await.unwrap();
        let written = write(dir.path(), Grouping::default()).await.unwrap();
        assert!(written.grouping.groups.is_empty());
        assert_eq!(read(dir.path()).await, Grouping::default());
    }
    /// **What is recorded is a copy the size it is drawn at**, not the generation: Home draws
    /// every icon on every open, and a generation is megabytes. The original stays put.
    #[tokio::test]
    async fn an_offered_icon_is_filed_as_an_icon_sized_square() {
        let dir = tempfile::tempdir().unwrap();
        task(dir.path(), "kt8-046").await;
        let original = drawn(dir.path(), "generated/2026-09-17/091634-a-microphone.png").await;
        let written =
            write(dir.path(), Grouping { groups: vec![with_icon(group("KTV", &["kt8-046"]), &original)] })
                .await
                .unwrap();
        let icon = written.grouping.groups[0].icon.clone().unwrap();
        assert!(icon.starts_with("drive/home/icons/"), "{icon}");
        let copy = media::resolve_ref(dir.path(), &icon).await.unwrap();
        let img = image::open(copy).unwrap();
        assert_eq!((img.width(), img.height()), (ICON_PX, ICON_PX));
        assert!(media::resolve_ref(dir.path(), &original).await.is_some(), "the original is left where it was made");
        assert!(written.iconless.is_empty() && written.refused_icons.is_empty());

        // Handing back what `groups.json` already holds files nothing new.
        let again = write(dir.path(), Grouping { groups: vec![with_icon(group("KTV", &["kt8-046"]), &icon)] })
            .await
            .unwrap();
        assert_eq!(again.grouping.groups[0].icon.as_deref(), Some(icon.as_str()));
    }

    /// **Rearranging is not redrawing.** The arrangement is replaced whole every pass, so an
    /// icon the writer did not repeat is the label's, not lost — and a label that is new is
    /// new, whatever it replaced.
    #[tokio::test]
    async fn a_rewrite_that_leaves_the_icon_out_keeps_the_one_the_label_had() {
        let dir = tempfile::tempdir().unwrap();
        task(dir.path(), "kt8-046").await;
        task(dir.path(), "vocabulary-book").await;
        let original = drawn(dir.path(), "generated/2026-09-17/a-microphone.png").await;
        let first =
            write(dir.path(), Grouping { groups: vec![with_icon(group("KTV", &["kt8-046"]), &original)] })
                .await
                .unwrap();
        let icon = first.grouping.groups[0].icon.clone();

        let rewritten = write(
            dir.path(),
            Grouping { groups: vec![group("学习类", &["vocabulary-book"]), group("KTV", &["kt8-046"])] },
        )
        .await
        .unwrap();
        assert_eq!(rewritten.grouping.groups[1].icon, icon);
        assert_eq!(rewritten.grouping.groups[0].icon, None);
        assert_eq!(rewritten.iconless, ["学习类"]);
        assert_eq!(read(dir.path()).await.groups[1].icon, icon);
    }

    /// An inner group's icon is filed, kept and reported exactly as an outer one's is.
    #[tokio::test]
    async fn an_inner_group_keeps_its_icon_across_a_rewrite_like_any_other() {
        let dir = tempfile::tempdir().unwrap();
        task(dir.path(), "rollup").await;
        task(dir.path(), "site-survey").await;
        let original = drawn(dir.path(), "generated/a-crane.png").await;
        let first = write(
            dir.path(),
            Grouping {
                groups: vec![holding(
                    "Client work",
                    &["rollup"],
                    vec![with_icon(group("Site", &["site-survey"]), &original)],
                )],
            },
        )
        .await
        .unwrap();
        let icon = first.grouping.groups[0].groups[0].icon.clone();
        assert!(icon.as_deref().is_some_and(|i| i.starts_with("drive/home/icons/")), "{icon:?}");
        assert_eq!(first.iconless, ["Client work"]);

        let rewritten = write(
            dir.path(),
            Grouping {
                groups: vec![holding("Client work", &["rollup"], vec![group("Site", &["site-survey"])])],
            },
        )
        .await
        .unwrap();
        assert_eq!(rewritten.grouping.groups[0].groups[0].icon, icon);
    }

    /// A kept icon whose file has gone is not kept: the label is back on the default, and the
    /// receipt names it, which is what gets it drawn again.
    #[tokio::test]
    async fn a_kept_icon_whose_file_is_gone_is_the_default_again() {
        let dir = tempfile::tempdir().unwrap();
        task(dir.path(), "kt8-046").await;
        let original = drawn(dir.path(), "generated/a-microphone.png").await;
        let first =
            write(dir.path(), Grouping { groups: vec![with_icon(group("KTV", &["kt8-046"]), &original)] })
                .await
                .unwrap();
        let copy = media::resolve_ref(dir.path(), first.grouping.groups[0].icon.as_deref().unwrap())
            .await
            .unwrap();
        tokio::fs::remove_file(copy).await.unwrap();
        let written = write(dir.path(), Grouping { groups: vec![group("KTV", &["kt8-046"])] }).await.unwrap();
        assert_eq!(written.grouping.groups[0].icon, None);
        assert_eq!(written.iconless, ["KTV"]);
    }

    /// **An icon that cannot be used is refused by itself.** The arrangement is the part the
    /// person asked for; a bad picture must not cost them it, nor the icon the label had.
    #[tokio::test]
    async fn an_unusable_icon_is_refused_and_the_arrangement_still_lands() {
        let dir = tempfile::tempdir().unwrap();
        task(dir.path(), "kt8-046").await;
        let original = drawn(dir.path(), "generated/a-microphone.png").await;
        let first =
            write(dir.path(), Grouping { groups: vec![with_icon(group("KTV", &["kt8-046"]), &original)] })
                .await
                .unwrap();
        let kept = first.grouping.groups[0].icon.clone();
        let notes = media::drive_root(dir.path()).join("notes.md");
        tokio::fs::write(&notes, "not a picture").await.unwrap();

        for offered in ["vision/2026-09-17/09/00-00.jpg", "drive/generated/never-made.png", "drive/notes.md"] {
            let written =
                write(dir.path(), Grouping { groups: vec![with_icon(group("KTV", &["kt8-046"]), offered)] })
                    .await
                    .unwrap();
            assert_eq!(written.grouping.groups[0].members, ["kt8-046"]);
            assert_eq!(written.grouping.groups[0].icon, kept, "{offered}");
            assert_eq!(written.refused_icons.len(), 1, "{offered}");
            assert!(written.refused_icons[0].starts_with("KTV: "), "{:?}", written.refused_icons);
        }
    }

    /// The ref a writer is told to draw from has to be there, and has to be this build's.
    #[tokio::test]
    async fn the_anchor_is_filed_as_the_default_icon_and_refiled_when_it_differs() {
        let dir = tempfile::tempdir().unwrap();
        file_icon_anchor(dir.path()).await.unwrap();
        let path = media::resolve_ref(dir.path(), ICON_ANCHOR_REF).await.unwrap();
        assert_eq!(tokio::fs::read(&path).await.unwrap(), DEFAULT_ICON);
        tokio::fs::write(&path, b"an older picture").await.unwrap();
        file_icon_anchor(dir.path()).await.unwrap();
        assert_eq!(tokio::fs::read(&path).await.unwrap(), DEFAULT_ICON);
    }
}
