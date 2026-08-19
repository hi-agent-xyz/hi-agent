//! `proactivity.md` — the learned read on what the agent's words have earned.
//!
//! A single derived file: which kinds of thing the person was glad to have been told and
//! which cost them more to read than they were worth, distilled by the reflection
//! ("sleep") pass from how the agent's own utterances landed. Reaction is tools-off and
//! cannot open a file, so it arrives *projected* into the window ([`super::snapshot`]),
//! never as a path. Rewritten wholesale by the reflection pass, never patched, and absent
//! until the first word has been judged.
//!
//! **It is not only about breaking silences.** It began as a read on speaking up
//! unprompted, which covered the smallest slice of what the agent says: everything said
//! with the floor already its own — replies, mid-flight progress, hand-backs — sat outside
//! the one artifact in the system that learns what its words cost. The standings are
//! unchanged; what widened is the set of subjects they can stand over.

use std::path::Path;

use super::layout;

/// The current read, or `None` when nothing has been recorded yet — so the reflection
/// pass starts a fresh file, and a subject with no line has no record, which is not
/// permission.
pub async fn read(data_dir: &Path) -> anyhow::Result<Option<String>> {
    match tokio::fs::read_to_string(layout::proactivity_path(data_dir)).await {
        Ok(s) => Ok(Some(s)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

/// Replace `proactivity.md` with `content` wholesale — the reflection pass
/// regenerates the whole file each time, it never patches.
pub async fn write(data_dir: &Path, content: &str) -> anyhow::Result<()> {
    let path = layout::proactivity_path(data_dir);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&path, content).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn read_absent_is_none_then_round_trips_after_write() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read(dir.path()).await.unwrap().is_none());
        write(dir.path(), "## family-reminders — asked for one twice; keep bringing them\n").await.unwrap();
        assert_eq!(read(dir.path()).await.unwrap().as_deref(), Some("## family-reminders — asked for one twice; keep bringing them\n"));
        // Regenerate wholesale, not patched.
        write(dir.path(), "## oil-price — brushed aside twice; do not raise it\n").await.unwrap();
        assert_eq!(read(dir.path()).await.unwrap().as_deref(), Some("## oil-price — brushed aside twice; do not raise it\n"));
    }
}
