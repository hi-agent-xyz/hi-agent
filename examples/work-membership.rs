//! Apply only reviewed membership evidence; no title, system or grouping inference.
use std::path::PathBuf;

use hi_agent::mind::memory::{tasks, work};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Change {
    subject: String,
    expected_revision: u64,
    work: work::Input,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        args.len() == 2 || (args.len() == 3 && args[2] == "--apply"),
        "usage: cargo run --example work-membership -- <data-dir> <reviewed-manifest.json> [--apply]; stop the core before applying"
    );
    let dir = PathBuf::from(&args[0]);
    let changes: Vec<Change> = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    let mut seen = std::collections::HashSet::new();
    for change in &changes {
        anyhow::ensure!(
            !change.subject.is_empty() && !change.subject.contains(['/', '\\']),
            "name a stable task subject, not a path"
        );
        anyhow::ensure!(
            seen.insert(&change.subject),
            "duplicate manifest subject `{}`",
            change.subject
        );
        work::validate(&dir, &change.work).await?;
        let row = tasks::read_task(&dir, &change.subject)
            .await?
            .ok_or_else(|| anyhow::anyhow!("no task `{}`", change.subject))?;
        let current = row.work.as_ref().map_or(0, |w| w.revision);
        anyhow::ensure!(
            current == change.expected_revision,
            "stale manifest for `{}`: expected {}, current {current}",
            change.subject,
            change.expected_revision
        );
        println!(
            "review {}: {:?} -> {:?}, expected revision {current}",
            change.subject,
            row.work.as_ref().map(|w| &w.assignment),
            change.work.assignment
        );
    }
    if args.len() == 2 {
        println!(
            "Dry run: {} reviewed changes; nothing written. Continuation and protection constraints are checked again by the store when applying.",
            changes.len()
        );
        return Ok(());
    }
    for change in changes {
        match tasks::relate(&dir, &change.subject, change.work, change.expected_revision).await {
            Ok(revision) => println!("applied {} at revision {revision}", change.subject),
            Err(error) => anyhow::bail!(
                "stopped at {}: {error}; previous successful changes remain recorded",
                change.subject
            ),
        }
    }
    Ok(())
}
