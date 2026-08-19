use std::collections::HashMap;

use anyhow::Context;
use argus_redact_core::{PatternConfig, builtin_patterns, match_patterns};
use redact_core::{AnalyzerEngine, EntityType};
use serde::Serialize;
use serde_json::Value;

use super::store::SecretStore;

const SCORE_FLOOR: f32 = 0.6;
const MAX_TEXT_BYTES: usize = 1024 * 1024;

#[derive(Clone)]
pub struct SensitiveDataFilter {
    engine: AnalyzerEngine,
    store: SecretStore,
}

#[derive(Debug, Clone, Serialize)]
pub struct PrivacyFinding {
    pub entity_type: String,
    pub action: &'static str,
    pub start: usize,
    pub end: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Projection {
    pub text: String,
    pub findings: Vec<PrivacyFinding>,
}

#[derive(Clone)]
struct Candidate {
    start: usize,
    end: usize,
    entity_type: String,
    score: f32,
    secret: bool,
    existing_ref: Option<String>,
}

impl SensitiveDataFilter {
    pub fn new(store: SecretStore) -> Self {
        Self {
            engine: AnalyzerEngine::new(),
            store,
        }
    }

    pub fn project_text(&self, text: &str) -> anyhow::Result<Projection> {
        if is_media_data_url(text) {
            return Ok(Projection {
                text: text.to_string(),
                findings: Vec::new(),
            });
        }
        if text.len() > MAX_TEXT_BYTES {
            anyhow::bail!(
                "model-bound text exceeds the privacy scanner limit of {MAX_TEXT_BYTES} bytes"
            );
        }

        let mut candidates = self.known_secret_candidates(text)?;
        let entity_types = protected_entity_types();
        let analysis = self
            .engine
            .analyze_with_entities(text, &entity_types, None)
            .context("running maintained PII/secret detectors")?;
        candidates.extend(
            analysis
                .detected_entities
                .into_iter()
                .filter(|hit| hit.score >= SCORE_FLOOR)
                .map(|hit| Candidate {
                    start: hit.start,
                    end: hit.end,
                    entity_type: hit.entity_type.as_str().to_string(),
                    score: hit.score,
                    secret: is_secret_type(&hit.entity_type),
                    existing_ref: None,
                }),
        );
        candidates.extend(chinese_structured_candidates(text)?);
        let reserved = placeholder_spans(text);
        candidates.retain(|candidate| {
            reserved
                .iter()
                .all(|(start, end)| candidate.end <= *start || candidate.start >= *end)
        });

        let selected = resolve_overlaps(candidates);
        if selected.is_empty() {
            return Ok(Projection {
                text: text.to_string(),
                findings: Vec::new(),
            });
        }

        let mut output = String::with_capacity(text.len());
        let mut cursor = 0;
        let mut counts: HashMap<String, usize> = HashMap::new();
        let mut pii_replacements: HashMap<(String, String), String> = HashMap::new();
        let mut findings = Vec::with_capacity(selected.len());

        for candidate in selected {
            if candidate.start < cursor
                || candidate.end > text.len()
                || !text.is_char_boundary(candidate.start)
                || !text.is_char_boundary(candidate.end)
            {
                continue;
            }
            output.push_str(&text[cursor..candidate.start]);
            let value = &text[candidate.start..candidate.end];
            let reference = if candidate.secret {
                match candidate.existing_ref {
                    Some(reference) => Some(reference),
                    None => Some(self.store.upsert_detected(value, &candidate.entity_type)?),
                }
            } else {
                None
            };
            let replacement = match &reference {
                Some(reference) => format!("[SECRET_REF:{reference}]"),
                None => {
                    let key = (candidate.entity_type.clone(), value.to_string());
                    match pii_replacements.get(&key) {
                        Some(replacement) => replacement.clone(),
                        None => {
                            let count = counts.entry(candidate.entity_type.clone()).or_default();
                            *count += 1;
                            let replacement = format!("[PII:{}_{}]", candidate.entity_type, count);
                            pii_replacements.insert(key, replacement.clone());
                            replacement
                        }
                    }
                }
            };
            output.push_str(&replacement);
            findings.push(PrivacyFinding {
                entity_type: candidate.entity_type,
                action: if reference.is_some() {
                    "reference"
                } else {
                    "mask"
                },
                start: candidate.start,
                end: candidate.end,
                reference,
            });
            cursor = candidate.end;
        }
        output.push_str(&text[cursor..]);
        Ok(Projection {
            text: output,
            findings,
        })
    }

    pub fn project_json(&self, value: &mut Value) -> anyhow::Result<Vec<PrivacyFinding>> {
        let mut findings = Vec::new();
        self.project_json_inner(value, &mut findings)?;
        Ok(findings)
    }

    fn project_json_inner(
        &self,
        value: &mut Value,
        findings: &mut Vec<PrivacyFinding>,
    ) -> anyhow::Result<()> {
        match value {
            Value::String(text) => {
                let projection = self.project_text(text)?;
                *text = projection.text;
                findings.extend(projection.findings);
            }
            Value::Array(values) => {
                for value in values {
                    self.project_json_inner(value, findings)?;
                }
            }
            Value::Object(values) => {
                let original = std::mem::take(values);
                for (key, mut value) in original {
                    let projected_key = self.project_text(&key)?;
                    findings.extend(projected_key.findings);
                    self.project_json_inner(&mut value, findings)?;
                    let unique_key = unique_json_key(values, projected_key.text);
                    values.insert(unique_key, value);
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
        Ok(())
    }

    fn known_secret_candidates(&self, text: &str) -> anyhow::Result<Vec<Candidate>> {
        let mut candidates = Vec::new();
        for stored in self.store.active_values()? {
            for (start, _) in text.match_indices(&stored.value) {
                candidates.push(Candidate {
                    start,
                    end: start + stored.value.len(),
                    entity_type: "GENERIC_SECRET".to_string(),
                    score: 1.0,
                    secret: true,
                    existing_ref: Some(stored.reference.clone()),
                });
            }
        }
        Ok(candidates)
    }
}

fn unique_json_key(values: &serde_json::Map<String, Value>, key: String) -> String {
    if !values.contains_key(&key) {
        return key;
    }
    for suffix in 2.. {
        let candidate = format!("{key}#{suffix}");
        if !values.contains_key(&candidate) {
            return candidate;
        }
    }
    unreachable!("an unused JSON key suffix always exists")
}

fn protected_entity_types() -> Vec<EntityType> {
    use EntityType::*;
    vec![
        EmailAddress,
        PhoneNumber,
        IpAddress,
        CreditCard,
        Iban,
        IbanCode,
        UsBankNumber,
        UsSsn,
        UsDriverLicense,
        UsPassport,
        UkNhs,
        UkNino,
        UkPostcode,
        UkDriverLicense,
        UkPassportNumber,
        UkPhoneNumber,
        UkMobileNumber,
        MedicalLicense,
        MedicalRecordNumber,
        PassportNumber,
        PoBox,
        MacAddress,
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

fn is_secret_type(entity_type: &EntityType) -> bool {
    entity_type.is_named_secret() || *entity_type == EntityType::GenericSecret
}

fn chinese_structured_candidates(text: &str) -> anyhow::Result<Vec<Candidate>> {
    const TYPES: &[&str] = &[
        "phone",
        "id_number",
        "bank_card",
        "passport",
        "license_plate",
    ];
    let patterns = builtin_patterns("zh")
        .iter()
        .filter(|pattern| TYPES.contains(&pattern.type_.as_str()))
        .map(|pattern| PatternConfig {
            type_: pattern.type_.clone(),
            pattern: pattern.pattern.clone(),
            check_context: pattern.check_context,
            group: pattern.group.clone(),
            validator: pattern.validator.clone(),
        })
        .collect::<Vec<_>>();
    let matches = match_patterns(text, &patterns)
        .map_err(|error| anyhow::anyhow!("running maintained Chinese PII detectors: {error}"))?;
    Ok(matches
        .into_iter()
        .filter(|hit| hit.confidence >= 1.0)
        .map(|hit| Candidate {
            start: char_to_byte(text, hit.start),
            end: char_to_byte(text, hit.end),
            entity_type: format!("CN_{}", hit.type_.to_ascii_uppercase()),
            score: hit.confidence as f32,
            secret: false,
            existing_ref: None,
        })
        .collect())
}

fn char_to_byte(text: &str, char_offset: usize) -> usize {
    text.char_indices()
        .nth(char_offset)
        .map(|(offset, _)| offset)
        .unwrap_or(text.len())
}

fn resolve_overlaps(mut candidates: Vec<Candidate>) -> Vec<Candidate> {
    candidates.retain(|candidate| candidate.start < candidate.end);
    candidates.sort_by(|a, b| {
        candidate_priority(b)
            .cmp(&candidate_priority(a))
            .then_with(|| b.score.total_cmp(&a.score))
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

fn candidate_priority(candidate: &Candidate) -> u8 {
    match (candidate.secret, candidate.existing_ref.is_some()) {
        (true, true) => 3,
        (true, false) => 2,
        (false, _) => 1,
    }
}

fn is_media_data_url(text: &str) -> bool {
    ["data:image/", "data:audio/", "data:video/"]
        .iter()
        .any(|prefix| text.starts_with(prefix))
}

fn placeholder_spans(text: &str) -> Vec<(usize, usize)> {
    const PREFIXES: &[&str] = &["[SECRET_REF:drive/accounts/secrets/", "[PII:"];
    let mut spans = Vec::new();
    for prefix in PREFIXES {
        let mut cursor = 0;
        while let Some(relative) = text[cursor..].find(prefix) {
            let start = cursor + relative;
            let Some(close) = text[start..].find(']') else {
                break;
            };
            let end = start + close + 1;
            spans.push((start, end));
            cursor = end;
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter() -> (tempfile::TempDir, SensitiveDataFilter) {
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::open(dir.path()).unwrap();
        (dir, SensitiveDataFilter::new(store))
    }

    #[test]
    fn secrets_become_stable_non_bearer_references() {
        let (_dir, filter) = filter();
        let key = [
            "sk-proj-",
            "abcdefghij_klmnopqrst-uvwxyz0123456789ABCDEFGHIJ",
        ]
        .concat();
        let input = format!("Use {key} with https://api.example.test/v1/me");

        let first = filter.project_text(&input).unwrap();
        assert!(!first.text.contains(&key));
        assert!(
            first
                .text
                .contains("[SECRET_REF:drive/accounts/secrets/openai-api-key.txt]")
        );
        let second = filter.project_text(&input).unwrap();
        assert_eq!(first.text, second.text);
        assert_eq!(first.findings[0].reference, second.findings[0].reference);
        let idempotent = filter.project_text(&first.text).unwrap();
        assert_eq!(
            idempotent.text, first.text,
            "drive-file references must survive later turns"
        );
        assert!(idempotent.findings.is_empty());
    }

    #[test]
    fn pii_is_masked_without_ner() {
        let (_dir, filter) = filter();
        let input = "Email alice@example.com, phone 555-123-4567, 身份证 11010519491231002X";
        let projected = filter.project_text(input).unwrap();
        assert!(!projected.text.contains("alice@example.com"));
        assert!(!projected.text.contains("555-123-4567"));
        assert!(!projected.text.contains("11010519491231002X"));
        assert!(projected.text.contains("[PII:EMAIL_ADDRESS_1]"));
        assert!(projected.text.contains("[PII:CN_ID_NUMBER_1]"));
    }

    #[test]
    fn repeated_pii_uses_one_stable_placeholder_per_projection() {
        let (_dir, filter) = filter();
        let projected = filter
            .project_text("alice@example.com then alice@example.com, not bob@example.com")
            .unwrap();
        assert_eq!(projected.text.matches("[PII:EMAIL_ADDRESS_1]").count(), 2);
        assert_eq!(projected.text.matches("[PII:EMAIL_ADDRESS_2]").count(), 1);
    }

    #[test]
    fn ordinary_code_and_urls_survive() {
        let (_dir, filter) = filter();
        let input =
            "GET https://api.example.test/v1/items; revision=550e8400-e29b-41d4-a716-446655440000";
        let projected = filter.project_text(input).unwrap();
        assert_eq!(projected.text, input);
        assert!(projected.findings.is_empty());
    }

    #[test]
    fn recursive_json_projection_reaches_tool_results() {
        let (_dir, filter) = filter();
        let key = ["ghp_", "abcdefghijklmnopqrstuvwxyz0123456789"].concat();
        let mut value = serde_json::json!({
            "input": [{
                "type": "function_call_output",
                "output": { "stdout": format!("token={key}") }
            }]
        });
        filter.project_json(&mut value).unwrap();
        assert!(!value.to_string().contains(&key));
        assert!(value.to_string().contains("SECRET_REF"));
    }

    #[test]
    fn recursive_json_projection_reaches_dynamic_object_keys_without_dropping_values() {
        let (_dir, filter) = filter();
        let mut value = serde_json::json!({
            "alice@example.com": 1,
            "bob@example.com": 2,
        });
        filter.project_json(&mut value).unwrap();
        let encoded = value.to_string();
        assert!(!encoded.contains("alice@example.com"));
        assert!(!encoded.contains("bob@example.com"));
        assert_eq!(value.as_object().unwrap().len(), 2);
        assert!(encoded.contains("[PII:EMAIL_ADDRESS_1]"));
        assert!(encoded.contains("[PII:EMAIL_ADDRESS_1]#2"));
    }
}
