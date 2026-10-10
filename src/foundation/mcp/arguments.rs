//! Narrow, schema-directed repair of stringified structured tool arguments.
//! This is not validation: the tool still checks every repaired argument.

use serde_json::Value;

const MAX_DEPTH: usize = 64;
const MAX_NODES: usize = 100_000;

pub(super) fn normalize(args: &Value, schema: &Value) -> Result<(Value, Vec<String>), String> {
    let mut args = args.clone();
    let mut repaired = Vec::new();
    let mut remaining = MAX_NODES;
    visit(&mut args, schema, "$", 0, &mut remaining, &mut repaired)?;
    Ok((args, repaired))
}

fn permits(schema: &Value, kind: &str) -> bool {
    match schema.get("type") {
        Some(Value::String(t)) => t == kind,
        Some(Value::Array(types)) => types.iter().any(|t| t.as_str() == Some(kind)),
        _ => false,
    }
}

fn visit(
    value: &mut Value,
    schema: &Value,
    path: &str,
    depth: usize,
    remaining: &mut usize,
    repaired: &mut Vec<String>,
) -> Result<(), String> {
    if depth > MAX_DEPTH || *remaining == 0 {
        return Err(format!(
            "{path}: structured arguments exceed normalization limits"
        ));
    }
    *remaining -= 1;

    let array = permits(schema, "array");
    let object = permits(schema, "object");
    // A string/object union deliberately accepts prose; never reinterpret it.
    if !permits(schema, "string") && (array || object) {
        if let Value::String(text) = value {
            let decoded: Value = serde_json::from_str(text).map_err(|_| {
                format!("{path}: expected a JSON array/object, but the string is not valid JSON")
            })?;
            if !(array && decoded.is_array() || object && decoded.is_object()) {
                return Err(format!(
                    "{path}: the JSON string must decode once to the declared array/object type"
                ));
            }
            *value = decoded;
            repaired.push(path.to_owned());
        }
    }

    match value {
        Value::Object(fields) if object => {
            for (key, value) in fields {
                let child = schema
                    .get("properties")
                    .and_then(|p| p.get(key))
                    .or_else(|| schema.get("additionalProperties").filter(|s| s.is_object()));
                if let Some(child) = child {
                    let key = key.replace('~', "~0").replace('/', "~1");
                    visit(
                        value,
                        child,
                        &format!("{path}/{key}"),
                        depth + 1,
                        remaining,
                        repaired,
                    )?;
                }
            }
        }
        Value::Array(items) if array => {
            if let Some(child) = schema.get("items").filter(|s| s.is_object()) {
                for (i, value) in items.iter_mut().enumerate() {
                    visit(
                        value,
                        child,
                        &format!("{path}/{i}"),
                        depth + 1,
                        remaining,
                        repaired,
                    )?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> Value {
        json!({"type": "object", "properties": {
            "text": {"type": "string"},
            "items": {"type": "array", "items": {"type": "object", "properties": {
                "tags": {"type": "array", "items": {"type": "string"}},
                "text": {"type": "string"}
            }}},
            "headers": {"type": "object", "additionalProperties": {"type": "string"}},
            "maps": {"type": "object", "additionalProperties": {"type": "array", "items": {"type": "string"}}},
            "ambiguous": {"type": ["string", "object"]},
            "nullable": {"type": ["array", "null"], "items": {"type": "string"}},
            "number": {"type": "number"}, "flag": {"type": "boolean"}
        }})
    }

    #[test]
    fn repairs_root_nested_arrays_objects_and_map_values() {
        let args = json!({
            "items": json!([json!({"tags": "[\"a\"]", "text": "[]"}).to_string()]).to_string(),
            "headers": "{\"X-Test\":\"[]\"}",
            "maps": "{\"a\":\"[\\\"b\\\"]\"}"
        });
        let (out, paths) = normalize(&json!(args.to_string()), &schema()).unwrap();
        assert_eq!(
            out,
            json!({"items": [{"tags": ["a"], "text": "[]"}], "headers": {"X-Test": "[]"}, "maps": {"a": ["b"]}})
        );
        assert_eq!(
            paths,
            vec![
                "$",
                "$/headers",
                "$/items",
                "$/items/0",
                "$/items/0/tags",
                "$/maps",
                "$/maps/a"
            ]
        );
    }

    #[test]
    fn preserves_text_unions_scalars_and_undeclared_fields() {
        let args = json!({"text": "{\"a\":1}", "ambiguous": "{}", "unknown": "[]", "number": "42", "flag": "true", "nullable": null});
        let (out, paths) = normalize(&args, &schema()).unwrap();
        assert_eq!(out, args);
        assert!(paths.is_empty());
        assert_eq!(
            normalize(&json!({"nullable": "[]"}), &schema()).unwrap().0,
            json!({"nullable": []})
        );
    }

    #[test]
    fn native_structures_are_unchanged() {
        let args =
            json!({"items": [{"tags": ["{}", "[]"], "text": "null"}], "headers": {"X-Test": "{}"}});
        let (out, paths) = normalize(&args, &schema()).unwrap();
        assert_eq!(out, args);
        assert!(paths.is_empty());
    }

    #[test]
    fn actual_prepare_schema_repairs_nested_fields_without_changing_actions() {
        let schema = super::super::prepare_tool()["inputSchema"].clone();
        let native = json!({"matter": "result", "branches": [
            {"when": "finished", "actions": [{"do": "say", "text": "[]"}]}
        ]});
        let encoded = json!({"matter": "result", "branches": json!([
            {"when": "finished", "actions": json!([
                {"do": "say", "text": "[]"}
            ]).to_string()}
        ]).to_string()});
        let (out, paths) = normalize(&encoded, &schema).unwrap();
        assert_eq!(out, native);
        assert_eq!(paths, vec!["$/branches", "$/branches/0/actions"]);
        for branches in [
            json!([{"when": "finished", "actions": [{"do": "shell"}]}]),
            json!([{"when": "finished", "actions": []}]),
            json!([{"when": "finished", "actions": [{"do": "say", "text": "x".repeat(401)}]}]),
        ] {
            let native = json!({"matter": "result", "branches": branches});
            let encoded = json!({"matter": "result", "branches": branches.to_string()});
            let (out, _) = normalize(&encoded, &schema).unwrap();
            assert_eq!(out, native, "repair must not sanitize an invalid action");
        }
    }

    #[test]
    fn actual_tool_schemas_repair_more_than_branches() {
        let http = super::super::http_request_tool();
        let args =
            json!({"url": "https://example.com", "headers": "{\"X-Test\":\"[]\"}", "body": "{}"});
        let (out, paths) = normalize(&args, &http["inputSchema"]).unwrap();
        assert_eq!(out["headers"], json!({"X-Test": "[]"}));
        assert_eq!(out["body"], "{}");
        assert_eq!(paths, vec!["$/headers"]);

        let system = super::super::system_one_tool();
        let args = json!({"state": "{}", "questions": "{\"q\":\"{\\\"type\\\":\\\"noul\\\",\\\"instructions\\\":\\\"[]\\\"}\"}"});
        let (out, paths) = normalize(&args, &system["inputSchema"]).unwrap();
        assert_eq!(
            out,
            json!({"state": "{}", "questions": {"q": {"type": "noul", "instructions": "[]"}}})
        );
        assert_eq!(paths, vec!["$/questions", "$/questions/q"]);
    }

    #[test]
    fn rejects_invalid_wrong_type_and_repeated_encoding_without_echoing_content() {
        for text in ["[secret,", "{}", "null", "42", "\"[]\""] {
            let err = normalize(&json!({"items": text}), &schema()).unwrap_err();
            assert!(err.starts_with("$/items:"), "{err}");
            assert!(!err.contains("secret"));
        }
        let err = normalize(&json!({"items": [{"tags": "{}"}]}), &schema()).unwrap_err();
        assert!(err.starts_with("$/items/0/tags:"), "{err}");
        assert_eq!(
            normalize(&json!({"items": "[]"}), &schema()).unwrap().0,
            json!({"items": []})
        );
    }

    #[test]
    fn schemas_without_a_structured_type_are_not_guessed() {
        for schema in [
            json!({}),
            json!({"anyOf": [{"type": "object"}, {"type": "string"}]}),
        ] {
            assert_eq!(
                normalize(&json!("{}"), &schema).unwrap(),
                (json!("{}"), vec![])
            );
        }
    }

    #[test]
    fn bounds_recursive_work() {
        let mut schema = json!({"type": "object"});
        let mut value = json!({});
        for _ in 0..MAX_DEPTH + 2 {
            schema = json!({"type": "object", "properties": {"child": schema}});
            value = json!({"child": value});
        }
        assert!(normalize(&value, &schema).unwrap_err().contains("limits"));
        let schema = json!({"type": "array", "items": {"type": "string"}});
        let value = Value::Array(vec![json!("x"); MAX_NODES]);
        assert!(normalize(&value, &schema).unwrap_err().contains("limits"));
    }
}
