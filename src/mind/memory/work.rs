//! Durable work membership, independent of a window's classification and coordinates.
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::facets;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum Assignment {
    Pending {},
    Independent {},
    Project {
        project: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        continues: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Context,
    User,
}

/// The caller supplies evidence, not a model confidence score. Revisions are store-owned.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub assignment: Assignment,
    pub source: Source,
    pub basis: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Relation {
    pub assignment: Assignment,
    pub source: Source,
    pub basis: String,
    pub revision: u64,
}

pub fn decode(raw: &str) -> Option<Relation> {
    let value: Relation = serde_json::from_str(raw).ok()?;
    if value.revision == 0 || !valid_basis(&value.basis) {
        return None;
    }
    if let Assignment::Project { project, continues } = &value.assignment {
        if project.is_empty() || facets::slug(project) != *project {
            return None;
        }
        if continues
            .as_ref()
            .is_some_and(|s| s.is_empty() || facets::slug(s) != *s)
        {
            return None;
        }
    }
    Some(value)
}

impl Relation {
    pub fn project(&self) -> Option<&str> {
        match &self.assignment {
            Assignment::Project { project, .. } => Some(project),
            _ => None,
        }
    }

    pub fn summary(&self) -> String {
        let place = match &self.assignment {
            Assignment::Pending {} => "membership pending".to_owned(),
            Assignment::Independent {} => "independent work".to_owned(),
            Assignment::Project { project, continues } => format!(
                "project: {project}{}",
                continues
                    .as_ref()
                    .map(|s| format!("; continues task: {s}"))
                    .unwrap_or_default()
            ),
        };
        format!(
            "{place}; revision: {}; source: {:?}; basis: {}",
            self.revision, self.source, self.basis
        )
    }
}

pub async fn validate(data_dir: &Path, input: &Input) -> anyhow::Result<()> {
    anyhow::ensure!(
        valid_basis(&input.basis),
        "work membership needs one line of evidence or a pending reason, at most 1000 characters"
    );
    if let Assignment::Project { project, continues } = &input.assignment {
        anyhow::ensure!(
            !project.is_empty() && facets::slug(project) == *project,
            "project must be an existing stable projects/ subject, not a label or path"
        );
        anyhow::ensure!(
            facets::facet_exists(data_dir, "projects", project).await,
            "no project record for `{project}`; write projects/{project} first, or record membership as pending"
        );
        if let Some(subject) = continues {
            anyhow::ensure!(
                !subject.is_empty() && facets::slug(subject) == *subject,
                "continues must name a task subject"
            );
        }
    }
    Ok(())
}

fn valid_basis(basis: &str) -> bool {
    !basis.trim().is_empty() && basis.chars().count() <= 1000 && !basis.contains(['\n', '\r'])
}

pub fn relation(input: Input, revision: u64) -> Relation {
    Relation {
        assignment: input.assignment,
        source: input.source,
        basis: input.basis.trim().to_owned(),
        revision,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assignment_rejects_ambiguous_fields_and_missing_project() {
        for raw in [
            r#"{"state":"pending","project":"p"}"#,
            r#"{"state":"independent","continues":"earlier"}"#,
            r#"{"state":"project"}"#,
        ] {
            assert!(serde_json::from_str::<Assignment>(raw).is_err(), "{raw}");
        }
    }

    #[tokio::test]
    async fn stored_and_new_evidence_share_the_same_validation() {
        let dir = tempfile::tempdir().unwrap();
        for basis in ["".into(), "a\nb".into(), "a\rb".into(), "x".repeat(1001)] {
            let input = Input {
                assignment: Assignment::Pending {},
                source: Source::Context,
                basis,
            };
            assert!(validate(dir.path(), &input).await.is_err());
            assert!(decode(&serde_json::to_string(&relation(input, 1)).unwrap()).is_none());
        }
    }
}
