//! Detection. Runs **once**, on what a person typed, and nowhere else.
//!
//! The output is not rewritten text — it is files. Each credential found in an
//! inbound message is written to `drive/accounts/secrets/<name>.txt` and the
//! message itself is journalled and shown verbatim. Substitution happens later
//! and elsewhere, by exact match against those files
//! ([`SecretStore::mask_known`]), at the one seam a model session is entered
//! through.
//!
//! **Secrets only.** PII is deliberately not detected: masking an address or a
//! number costs the agent the ability to do the thing it was asked to do, and
//! unlike a credential there is no file to hand it instead.

use anyhow::Context;
use redact_core::{AnalysisResult, AnalyzerEngine, EntityType};
use serde::Serialize;

use super::store::SecretStore;

const SCORE_FLOOR: f32 = 0.6;
const MAX_TEXT_BYTES: usize = 1024 * 1024;

#[derive(Clone)]
pub struct SensitiveDataFilter {
    engine: AnalyzerEngine,
    store: SecretStore,
}

/// One credential found in an inbound message, and the file it now lives in.
#[derive(Debug, Clone, Serialize)]
pub struct PrivacyFinding {
    pub entity_type: String,
    pub reference: String,
}

#[derive(Clone)]
struct Candidate {
    start: usize,
    end: usize,
    entity_type: String,
    score: f32,
}

impl SensitiveDataFilter {
    pub fn new(store: SecretStore) -> Self {
        Self {
            engine: AnalyzerEngine::new(),
            store,
        }
    }

    /// Find credentials in one inbound message and file each one.
    ///
    /// Returns what was filed, for the log. The caller keeps `text` as it stands:
    /// nothing about the journal, the conversation, or what the person sees is
    /// this function's business.
    ///
    /// A value already on file is recognised as itself and files nothing new, so
    /// sending the same key twice yields one file and one stable path.
    pub fn file_secrets(&self, text: &str) -> anyhow::Result<Vec<PrivacyFinding>> {
        if text.len() > MAX_TEXT_BYTES {
            // Detection is skipped, not fatal: this is an ingest-side scan of what
            // somebody typed, and refusing their message because it is long would
            // be a worse failure than not scanning it. Said out loud rather than
            // swallowed.
            tracing::warn!(
                len = text.len(),
                limit = MAX_TEXT_BYTES,
                "inbound message exceeds the secret-scanner limit; not scanned"
            );
            return Ok(Vec::new());
        }

        let stand_in = ascii_stand_in(text);
        let analysis = self.analyze(stand_in.as_deref().unwrap_or(text), &secret_entity_types())?;
        let candidates = analysis
            .detected_entities
            .into_iter()
            .filter(|hit| hit.score >= SCORE_FLOOR)
            .map(|hit| {
                let (start, end) = enclosing_chars(text, hit.start, hit.end);
                Candidate {
                    start,
                    end,
                    entity_type: hit.entity_type.as_str().to_string(),
                    score: hit.score,
                }
            })
            .collect::<Vec<_>>();

        let mut findings = Vec::new();
        for candidate in resolve_overlaps(candidates) {
            let Some(value) = text.get(candidate.start..candidate.end) else {
                continue;
            };
            findings.push(PrivacyFinding {
                reference: self.store.upsert_detected(value, &candidate.entity_type)?,
                entity_type: candidate.entity_type,
            });
        }
        Ok(findings)
    }

    /// `redact-core` 0.10.0 slices a +/-50-*byte* context window around every
    /// hit without checking char boundaries (`recognizers/pattern.rs:615`), so
    /// on multi-byte text it panics instead of returning an error.
    /// `ascii_stand_in` takes that cause away; this catches a panic anyway, so
    /// the next upstream one cannot unwind out through the axum handler and drop
    /// the connection. Analysis reads the engine without mutating it, so it
    /// stays usable afterwards.
    fn analyze(&self, text: &str, entity_types: &[EntityType]) -> anyhow::Result<AnalysisResult> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.engine.analyze_with_entities(text, entity_types, None)
        }))
        .map_err(|_| anyhow::anyhow!("the secret detector panicked on this text"))?
        .context("running maintained secret detectors")
    }
}

/// The credential types worth filing. Every one of them is a *bearer* value: it
/// grants access by itself, and the agent can use it from a file without reading
/// it. Nothing here identifies a person — see the module note.
fn secret_entity_types() -> Vec<EntityType> {
    use EntityType::*;
    vec![
        PrivateKey,
        JwtToken,
        AwsAccessKey,
        GithubToken,
        GitlabToken,
        SlackToken,
        SlackWebhook,
        StripeApiKey,
        GoogleApiKey,
        OpenAiApiKey,
        AnthropicApiKey,
        NpmToken,
        PyPiToken,
        SendGridApiKey,
        TwilioApiKey,
        TelegramBotToken,
        HashicorpVaultToken,
        DatabaseConnectionString,
        HuggingFaceToken,
        DatabricksToken,
        DigitalOceanToken,
        NotionApiKey,
        PerplexityApiKey,
        HttpBasicAuth,
        GenericSecret,
    ]
}

/// A byte-for-byte ASCII copy of `text`: every non-ASCII char's bytes become
/// newlines, one filler byte per original byte, so a hit's offsets are still
/// valid for `text`. Every credential form here is ASCII-structured, so nothing
/// is given up by scanning the stand-in — and two things are gained. The
/// byte-window panic above cannot fire, because every byte is now a char
/// boundary. And detection reaches a key that touches Chinese at all: `regex`'s
/// `\b` is Unicode-aware, so `是` is a word char and `我的key是sk-proj-…` matched
/// nothing before. `None` means `text` is already ASCII and can be scanned as it
/// stands.
fn ascii_stand_in(text: &str) -> Option<String> {
    if text.is_ascii() {
        return None;
    }
    let mut stand_in = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_ascii() {
            stand_in.push(ch);
        } else {
            for _ in 0..ch.len_utf8() {
                stand_in.push('\n');
            }
        }
    }
    Some(stand_in)
}

/// Widen a span from the stand-in — where every byte is a boundary — to whole
/// chars of `text`. A hit that reaches into filler would otherwise slice a char
/// in half.
fn enclosing_chars(text: &str, start: usize, end: usize) -> (usize, usize) {
    let mut start = start.min(text.len());
    let mut end = end.min(text.len());
    while start > 0 && !text.is_char_boundary(start) {
        start -= 1;
    }
    while end < text.len() && !text.is_char_boundary(end) {
        end += 1;
    }
    (start, end)
}

/// Keep the highest-scoring of any two hits that overlap, so one credential is
/// filed once rather than once per detector that recognised it.
fn resolve_overlaps(mut candidates: Vec<Candidate>) -> Vec<Candidate> {
    candidates.retain(|candidate| candidate.start < candidate.end);
    candidates.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| (b.end - b.start).cmp(&(a.end - a.start)))
            .then_with(|| a.start.cmp(&b.start))
    });
    let mut selected: Vec<Candidate> = Vec::new();
    for candidate in candidates {
        if selected
            .iter()
            .all(|kept| candidate.end <= kept.start || candidate.start >= kept.end)
        {
            selected.push(candidate);
        }
    }
    selected.sort_by_key(|candidate| candidate.start);
    selected
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter() -> (tempfile::TempDir, SensitiveDataFilter, SecretStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::open(dir.path()).unwrap();
        (dir, SensitiveDataFilter::new(store.clone()), store)
    }

    fn a_key() -> String {
        [
            "sk-proj-",
            "abcdefghij_klmnopqrst-uvwxyz0123456789ABCDEFGHIJ",
        ]
        .concat()
    }

    /// Ingest files the key and leaves the message alone; the seam substitutes.
    /// The two halves are separate on purpose — what the person sees is not what
    /// the model is handed.
    #[test]
    fn a_typed_key_is_filed_and_then_masked_by_path() {
        let (_dir, filter, store) = filter();
        let key = a_key();
        let typed = format!("use {key} against the staging API");

        let findings = filter.file_secrets(&typed).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].reference,
            "drive/accounts/secrets/openai-api-key.txt"
        );

        let masked = store.mask_known(&typed);
        assert!(!masked.contains(&key));
        assert_eq!(
            masked,
            "use ⟨secret: drive/accounts/secrets/openai-api-key.txt⟩ against the staging API"
        );
    }

    /// The whole reason masking is exact-match rather than re-detection: the same
    /// message is rendered into a prompt again on every later turn, through the
    /// journal snapshot, and must read identically each time.
    #[test]
    fn masking_the_same_message_twice_gives_the_same_text() {
        let (_dir, filter, store) = filter();
        let typed = format!("key {}", a_key());
        filter.file_secrets(&typed).unwrap();
        assert_eq!(store.mask_known(&typed), store.mask_known(&typed));
    }

    /// Sending it again must not fill the directory with copies, because the path
    /// is what the agent was told to use.
    #[test]
    fn the_same_key_twice_is_one_file() {
        let (_dir, filter, store) = filter();
        let key = a_key();
        let first = filter.file_secrets(&format!("here: {key}")).unwrap();
        let second = filter.file_secrets(&format!("again: {key}")).unwrap();
        assert_eq!(first[0].reference, second[0].reference);
        assert_eq!(store.active_values().unwrap().len(), 1);
    }

    /// PII is not this filter's business — masking it would cost the agent the
    /// ability to mail or call anyone, and there is no file to hand over instead.
    #[test]
    fn personal_details_are_not_filed_or_masked() {
        let (_dir, filter, store) = filter();
        let typed = "mail alice@example.com, call 555-123-4567, 身份证 11010519491231002X";
        assert!(filter.file_secrets(typed).unwrap().is_empty());
        assert_eq!(store.mask_known(typed), typed);
    }

    /// Ordinary text must survive untouched, or every prompt pays for this.
    #[test]
    fn ordinary_text_is_returned_borrowed_and_unchanged() {
        let (_dir, filter, store) = filter();
        let typed = "GET https://api.example.test/v1/items; rev=550e8400-e29b-41d4-a716-446655440000";
        assert!(filter.file_secrets(typed).unwrap().is_empty());
        assert!(matches!(store.mask_known(typed), std::borrow::Cow::Borrowed(_)));
    }

    /// The panic that `e195227` found, kept pinned: a key inside Chinese text is
    /// both detected and, because `\b` is Unicode-aware, detected *at all*.
    #[test]
    fn a_key_inside_chinese_text_is_found_without_panicking() {
        let (_dir, filter, store) = filter();
        let key = a_key();
        let typed = format!("这是我的密钥{key}，帮我测试一下接口能不能用，谢谢。");
        assert_eq!(filter.file_secrets(&typed).unwrap().len(), 1);
        let masked = store.mask_known(&typed);
        assert!(!masked.contains(&key));
        assert!(masked.contains("这是我的密钥⟨secret:"));
    }

    /// A short secret contained inside a longer one must not cut the longer one
    /// in half — the reason the cache is ordered longest-first.
    #[test]
    fn a_secret_inside_another_secret_masks_the_longer_one_whole() {
        let (_dir, _filter, store) = filter();
        store.upsert_detected("abcd1234", "GENERIC_SECRET").unwrap();
        store
            .upsert_detected("abcd1234efgh5678", "GENERIC_SECRET")
            .unwrap();
        let masked = store.mask_known("token=abcd1234efgh5678");
        assert_eq!(masked.matches("⟨secret:").count(), 1);
        assert!(!masked.contains("efgh5678"));
    }

    /// An oversized message is still delivered — scanning is skipped, not fatal.
    #[test]
    fn an_oversized_message_is_delivered_unscanned() {
        let (_dir, filter, _store) = filter();
        let huge = "x".repeat(MAX_TEXT_BYTES + 1);
        assert!(filter.file_secrets(&huge).unwrap().is_empty());
    }
}
