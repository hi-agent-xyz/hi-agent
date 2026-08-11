//! Output pacing — release one segment of a turn into ordered wire actions.
//!
//! The mind's output arrives as `say`/`show` tool calls the sequencer
//! ([`super::sequencer`]) feeds through here one at a time. These helpers keep a
//! view paced to its narration: [`view_emits`] flushes the pending spoken
//! sentence to TTS *before* showing the view, so it lands after the sentence
//! before it and right as the sentence after it begins synthesizing — never
//! racing ahead of already-produced speech. Both are pure (no `Reaction`), so the
//! release ordering is unit-testable; the sequencer performs the [`Emit`]s.

use std::time::Instant;

use crate::foundation::segment::{Segmenter, Terminator};
use crate::types::{ViewOp, ViewTraits};

/// One release action the policy decides on. `Speak` goes to TTS only (the raw
/// chunk is mirrored to /thought separately by the sequencer); `Show` is
/// compiled then sent to /view.
#[derive(Debug)]
pub(super) enum Emit {
    Speak(String),
    Show {
        id: String,
        op: ViewOp,
        source: String,
        traits: Option<ViewTraits>,
        view_ref: Option<String>,
    },
}

/// Coalesce spoken text into sentences for TTS. Pure: no side-effects, so the
/// release ordering is unit-testable without a `Reaction`.
pub(super) fn speak_emits(
    text: &str,
    splitter: &mut Segmenter<Terminator>,
    now: Instant,
) -> Vec<Emit> {
    splitter.commit(text, now).into_iter().map(Emit::Speak).collect()
}

/// Release a view, paced to its sentence: flush whatever spoken text the splitter
/// is still holding FIRST, so the view lands after the sentence before it and
/// right as the sentence after it begins synthesizing — never jumping ahead of
/// already-produced narration. Pure, for the same reason as [`speak_emits`].
pub(super) fn view_emits(
    splitter: &mut Segmenter<Terminator>,
    id: String,
    op: ViewOp,
    source: String,
    traits: Option<ViewTraits>,
    view_ref: Option<String>,
) -> Vec<Emit> {
    let mut out = Vec::new();
    if let Some(tail) = splitter.flush() {
        out.push(Emit::Speak(tail));
    }
    out.push(Emit::Show { id, op, source, traits, view_ref });
    out
}

#[cfg(test)]
mod release_tests {
    use super::*;

    /// Render the emit stream into a compact ordered transcript for assertion.
    fn trace(emits: &[Emit]) -> Vec<String> {
        emits
            .iter()
            .map(|e| match e {
                Emit::Speak(s) => format!("speak:{s}"),
                Emit::Show { source, .. } => format!("show:{source}"),
            })
            .collect()
    }

    #[test]
    fn view_is_paced_to_its_following_sentence() {
        // The core race fix: view1, narrate one, view2, narrate two — each view
        // emitted before its sentence, never both up front. Trailing spaces mirror
        // real LLM output so each sentence cuts cleanly on its terminator.
        let now = Instant::now();
        let mut sp = Segmenter::new(Terminator, now);
        let mut emits = Vec::new();
        let declared = Some(ViewTraits { owns_conversation: true });
        emits.extend(view_emits(&mut sp, "a".into(), ViewOp::Show, "c1".into(), declared, None));
        emits.extend(speak_emits("Narrate one. ", &mut sp, now));
        emits.extend(view_emits(&mut sp, "b".into(), ViewOp::Show, "c2".into(), None, None));
        emits.extend(speak_emits("Narrate two. ", &mut sp, now));
        if let Some(tail) = sp.flush() {
            emits.push(Emit::Speak(tail));
        }
        assert_eq!(
            trace(&emits),
            vec!["show:c1", "speak:Narrate one.", "show:c2", "speak:Narrate two."]
        );
        // What the view declared rides the emit untouched (and a view that
        // declared nothing stays None — host-owned captions).
        let traits_of = |want: &str| {
            emits.iter().find_map(|e| match e {
                Emit::Show { id, traits, .. } if id == want => Some(*traits),
                _ => None,
            })
        };
        assert_eq!(traits_of("a"), Some(declared));
        assert_eq!(traits_of("b"), Some(None));
    }

    #[test]
    fn view_flushes_a_preceding_partial_sentence_first() {
        // A sentence with no terminator before a view: the view flushes it as a
        // Speak BEFORE the Show, so it never jumps ahead of its narration.
        let now = Instant::now();
        let mut sp = Segmenter::new(Terminator, now);
        let mut emits = speak_emits("partial no period", &mut sp, now);
        emits.extend(view_emits(&mut sp, "a".into(), ViewOp::Show, "c1".into(), None, None));
        assert_eq!(trace(&emits), vec!["speak:partial no period", "show:c1"]);
    }
}
