//! Discovery categories are independent of project membership and carry their own evidence.
use serde::{Deserialize, Serialize};

use super::work::Source;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum Placement {
    Pending {},
    Category { path: Vec<String> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub placement: Placement,
    pub source: Source,
    pub basis: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Relation {
    pub placement: Placement,
    pub source: Source,
    pub basis: String,
    pub revision: u64,
}

pub fn validate(input: &Input) -> anyhow::Result<()> {
    anyhow::ensure!(
        !input.basis.trim().is_empty()
            && input.basis.chars().count() <= 1000
            && !input.basis.contains(['\n', '\r']),
        "classification needs one line of evidence or a pending reason, at most 1000 characters"
    );
    if let Placement::Category { path } = &input.placement {
        anyhow::ensure!(
            !path.is_empty() && path.len() <= 8,
            "category path must contain 1 to 8 labels"
        );
        let mut seen = std::collections::HashSet::new();
        for label in path {
            anyhow::ensure!(
                !label.trim().is_empty()
                    && label.trim() == label
                    && label.chars().count() <= 100
                    && !label.chars().any(char::is_control)
                    && seen.insert(label),
                "category labels must be unique, trimmed and at most 100 characters"
            );
        }
    }
    Ok(())
}

pub fn decode(raw: &str) -> Option<Relation> {
    let value: Relation = serde_json::from_str(raw).ok()?;
    if value.revision == 0 {
        return None;
    }
    validate(&Input {
        placement: value.placement.clone(),
        source: value.source,
        basis: value.basis.clone(),
    })
    .ok()?;
    Some(value)
}

impl Relation {
    pub fn summary(&self) -> String {
        let placement = match &self.placement {
            Placement::Pending {} => "classification pending".to_owned(),
            Placement::Category { path } => path.join(" > "),
        };
        format!(
            "{placement}; revision: {}; source: {:?}; basis: {}",
            self.revision, self.source, self.basis
        )
    }
}

pub fn relation(input: Input, revision: u64) -> Relation {
    Relation {
        placement: input.placement,
        source: input.source,
        basis: input.basis.trim().to_owned(),
        revision,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_paths_and_evidence_cannot_become_categories() {
        for path in [
            vec![],
            vec!["Life", "Life"],
            vec![" Life"],
            vec!["Life\nHealth"],
        ] {
            let input = Input {
                placement: Placement::Category {
                    path: path.into_iter().map(str::to_owned).collect(),
                },
                source: Source::Context,
                basis: "category evidence".into(),
            };
            assert!(validate(&input).is_err());
            assert!(decode(&serde_json::to_string(&relation(input, 1)).unwrap()).is_none());
        }
        assert!(
            serde_json::from_str::<Placement>(r#"{"state":"pending","path":["Life"]}"#).is_err()
        );
        assert!(serde_json::from_str::<Placement>(r#"{"state":"category"}"#).is_err());
    }
}
