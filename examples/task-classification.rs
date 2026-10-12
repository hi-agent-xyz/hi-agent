//! Reviewed offline category repair; dry run by default, never infer project membership.
use hi_agent::mind::memory::{classification, tasks};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Change {
    subject: String,
    expected_revision: u64,
    classification: classification::Input,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        args.len() == 2 || (args.len() == 3 && args[2] == "--apply"),
        "usage: cargo run --example task-classification -- <data-dir> <manifest.json> [--apply]; stop the core and back up before applying"
    );
    let dir = std::path::Path::new(&args[0]);
    let changes: Vec<Change> = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    let mut seen = std::collections::HashSet::new();
    for change in &changes {
        anyhow::ensure!(seen.insert(&change.subject), "duplicate manifest subject");
        classification::validate(&change.classification)?;
        let row = tasks::read_task(dir, &change.subject)
            .await?
            .ok_or_else(|| anyhow::anyhow!("no task `{}`", change.subject))?;
        let current = row.classification.as_ref().map_or(0, |c| c.revision);
        anyhow::ensure!(
            current == change.expected_revision,
            "stale classification for `{}`: current {current}",
            change.subject
        );
        if let Some(old) = &row.classification {
            anyhow::ensure!(
                old.source != hi_agent::mind::memory::work::Source::User
                    || old.placement == change.classification.placement
                    || change.classification.source == hi_agent::mind::memory::work::Source::User,
                "cannot overwrite a user category with context"
            );
        }
        println!(
            "review {}: {:?} -> {:?}, expected revision {current}",
            change.subject,
            row.classification.as_ref().map(|c| &c.placement),
            change.classification.placement
        );
    }
    if args.len() == 2 {
        println!("Dry run: {} changes; nothing written.", changes.len());
        return Ok(());
    }
    for change in changes {
        match tasks::classify(
            dir,
            &change.subject,
            change.classification,
            change.expected_revision,
        )
        .await
        {
            Ok(revision) => println!("applied {} at category revision {revision}", change.subject),
            Err(error) => anyhow::bail!(
                "stopped at {}: {error}; previous successful changes remain",
                change.subject
            ),
        }
    }
    Ok(())
}
