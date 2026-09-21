//! The gate on the home screen's names (`docs/arch/legibility.md` § *Home*): a group's label and
//! its note are on the person's screen every time they open it, and are written in one call that
//! replaces the whole arrangement — so every label or note that is new or changed since the
//! standing arrangement is read before it lands. The record's gate, pointed at names.

use std::path::Path;

use super::{Gate, Review, gate};
use crate::foundation::server::home::{Group, Grouping};
use crate::mind::memory::quality::{Scope, Surface};

/// The home screen's gate: `home_check` switches it off, `home_check_model` chooses its model.
pub(crate) const GATE: Gate = Gate {
    surface: Surface::Home,
    switch_key: "home_check",
    model_key: "home_check_model",
    budget_key: "home_check_budget_ms",
    rubric: crate::identity::judges::HOME,
};

/// The one key the arrangement is judged under: there is one arrangement, replaced whole.
const KEY: &str = "home";

/// Every group at every depth, outward from the core, as `(label, note)`.
fn names(groups: &[Group], out: &mut Vec<(String, Option<String>)>) {
    for g in groups {
        out.push((g.label.trim().to_owned(), g.note.as_deref().map(str::trim).filter(|n| !n.is_empty()).map(str::to_owned)));
        names(&g.groups, out);
    }
}

/// What in `proposed` is new or changed against `standing`: a label that was not there, or a
/// label whose note now reads differently. Reordering and membership are not names.
fn changed(standing: &Grouping, proposed: &Grouping) -> Vec<(String, Option<String>)> {
    let mut before = Vec::new();
    names(&standing.groups, &mut before);
    let mut after = Vec::new();
    names(&proposed.groups, &mut after);
    after.into_iter().filter(|name| !before.contains(name)).collect()
}

/// Decide whether `proposed` goes onto the home screen. `writer` is the task manager writing it.
pub async fn review(data_dir: &Path, writer: &str, standing: &Grouping, proposed: &Grouping) -> Review {
    let changed = changed(standing, proposed);
    if changed.is_empty() {
        return Review::Pass;
    }
    let (case, message) = case(standing, &changed);
    gate(data_dir, &GATE, writer, KEY, Scope::Label, case, message).await
}

fn line((label, note): &(String, Option<String>)) -> String {
    match note {
        Some(note) => format!("{label} — {note}"),
        None => label.clone(),
    }
}

/// What the judge reads after who the reader is: the names standing now, then what is about to
/// go up, numbered.
fn case(standing: &Grouping, changed: &[(String, Option<String>)]) -> (String, String) {
    let mut before = Vec::new();
    names(&standing.groups, &mut before);
    let before = before.iter().map(|n| format!("- {}", line(n))).collect::<Vec<_>>().join("\n");
    let about = changed.iter().enumerate().map(|(i, n)| format!("{}. {}", i + 1, line(n))).collect::<Vec<_>>().join("\n");
    let case = format!(
        "## The labels on their home screen now, each with its note\n{}\n\n## About to go up — new or changed\n{about}\n",
        if before.is_empty() { "(nothing)" } else { &before }
    );
    (case, about)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(label: &str, note: Option<&str>, inner: Vec<Group>) -> Group {
        Group { label: label.into(), note: note.map(Into::into), icon: None, members: vec![], groups: inner }
    }

    /// **Only what is new or reads differently is read**, at every depth; moving a group or its
    /// tasks around changes no name.
    #[test]
    fn only_new_or_changed_names_are_read() {
        let standing = Grouping { groups: vec![group("KNQ", Some("公司项目"), vec![group("赛马专家", None, vec![])])] };
        let reordered = Grouping { groups: vec![group("KNQ", Some("公司项目"), vec![group("赛马专家", None, vec![])])] };
        assert!(changed(&standing, &reordered).is_empty());

        let renamed = Grouping {
            groups: vec![group("KNQ", Some("他说的公司项目"), vec![group("赛马专家", None, vec![]), group("买鞋", None, vec![])])],
        };
        let got: Vec<String> = changed(&standing, &renamed).iter().map(line).collect();
        assert_eq!(got, vec!["KNQ — 他说的公司项目", "买鞋"]);
    }
}
