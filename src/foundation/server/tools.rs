//! The whole tool registry, in one response — inspection only.
//!
//! Unlike the other review surfaces, there is nothing here to correct. A tool surface
//! is compile-time code: [`tools_for_role`](crate::foundation::mcp::tools_for_role) is a
//! `match` over the role string, and every tool is a `json!` literal in
//! `foundation/mcp`. No state accumulates, so nothing can drift out of true and there is
//! no write verb to offer. This route exists to *read* what was compiled in.
//!
//! What it adds over the protocol: MCP's own `tools/list` answers for **one** role — the
//! caller's, taken from the `X-HI-Role` header on its `/mcp` connection — so a session
//! can only ever see its own surface, and comparing two rungs meant opening two sessions
//! with two headers, or reading the source. `GET /api/tools` returns every arm at once,
//! which is the form the question is actually asked in: what does Deliberation have that
//! Reaction doesn't, and who can dispatch work.
//!
//! Names and descriptions only. The input schemas are most of the bytes and none of the
//! answer — `say`'s schema is one string field, `act`'s is nine — so a review view that
//! carried them would bury the surface it is meant to show. Read the schema in
//! `foundation/mcp` when it matters.

use axum::Json;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::Value;

use crate::foundation::mcp::tools_for_role;

/// Every role `tools_for_role` recognises, **in the order its match arms appear** — so
/// the response reads in the same order as the code it describes.
///
/// `other` is the `_` fallback arm, and it is deliberately empty: every role hi-agent
/// opens is named above it, so reaching it means an unheadered or unknown session, and
/// the arm returns no tools rather than lending one an arbitrary kit. It is listed here
/// precisely so that emptiness is visible — the arm previously held the legacy agentic
/// reaction's toolset long after no live role mapped to it, and read as a live surface in
/// every review.
const ROLES: [&str; 6] = ["worker", "reflection", "cognition", "deliberation", "reaction", "other"];

/// The role a listed surface is served under. `other` is the `_` arm, reached with no
/// role at all.
fn role_arg(role: &str) -> Option<&str> {
    match role {
        "other" => None,
        named => Some(named),
    }
}

#[derive(Serialize)]
struct ToolDto {
    name: String,
    description: String,
}

#[derive(Serialize)]
struct RoleDto {
    role: &'static str,
    tools: Vec<ToolDto>,
}

/// `GET /api/tools` — every role's tool surface, names and descriptions.
pub async fn get_tools() -> Response {
    let roles: Vec<RoleDto> = ROLES.into_iter().map(surface).collect();
    Json(serde_json::json!({ "roles": roles })).into_response()
}

/// One role's surface, read out of the same function the MCP endpoint serves — not a
/// second list. A hand-kept copy here would be wrong the first time an arm changed, and
/// wrong invisibly, since this is the view people would check it against.
fn surface(role: &'static str) -> RoleDto {
    let tools = tools_for_role(role_arg(role))
        .iter()
        .map(|t| ToolDto {
            name: field(t, "name"),
            description: field(t, "description"),
        })
        .collect();
    RoleDto { role, tools }
}

/// One string field of a tool declaration, empty when absent. The declarations are
/// `json!` literals built by `mcp::tool`, so both fields are always there; this reads
/// them without letting a malformed one panic a review route.
fn field(tool: &Value, key: &str) -> String {
    tool.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every named role hands out at least one tool, and every tool is fully declared.
    /// An empty name is a tool the model cannot call; an empty description is one it
    /// cannot know when to call.
    #[test]
    fn every_named_role_yields_usable_tools() {
        for role in ROLES.into_iter().filter(|r| *r != "other") {
            let surface = surface(role);
            assert!(!surface.tools.is_empty(), "role `{role}` has no tools");
            for tool in &surface.tools {
                assert!(!tool.name.trim().is_empty(), "unnamed tool on role `{role}`");
                assert!(
                    !tool.description.trim().is_empty(),
                    "tool `{}` on role `{role}` has no description",
                    tool.name
                );
            }
        }
    }

    /// The `_` arm is empty on purpose. If this ever fails, somebody has handed an
    /// unheadered session a toolset again.
    #[test]
    fn the_fallback_arm_is_empty() {
        assert_eq!(role_arg("other"), None);
        assert!(surface("other").tools.is_empty());
    }

    /// A role cannot advertise the same name twice — the model would have no way to
    /// choose, and MCP clients key their tool table by name.
    #[test]
    fn no_role_advertises_a_duplicate_name() {
        for role in ROLES {
            let mut names: Vec<String> = surface(role).tools.into_iter().map(|t| t.name).collect();
            let total = names.len();
            names.sort();
            names.dedup();
            assert_eq!(names.len(), total, "role `{role}` advertises a duplicate tool name");
        }
    }

    /// The shape the view is written against, plus the two facts the whole endpoint is
    /// for: the surfaces genuinely differ per role, and only the standing rungs may
    /// dispatch work.
    #[test]
    fn the_response_is_keyed_by_role_and_the_surfaces_differ() {
        let roles: Vec<RoleDto> = ROLES.into_iter().map(surface).collect();
        let v = serde_json::json!({ "roles": roles });
        let arr = v["roles"].as_array().unwrap();
        assert_eq!(arr.len(), ROLES.len());
        assert_eq!(arr[0]["role"], "worker", "arms are listed in source order");
        assert!(arr[0]["tools"][0]["name"].is_string());

        let names = |role: &'static str| -> Vec<String> {
            surface(role).tools.into_iter().map(|t| t.name).collect()
        };
        assert!(names("reaction").contains(&"say".to_string()));
        assert!(!names("cognition").contains(&"say".to_string()), "only Reaction speaks");
        assert_eq!(names("deliberation"), vec!["send_message".to_string()]);
        for role in ["worker", "deliberation", "reaction"] {
            assert!(
                !names(role).contains(&"create_worker".to_string()),
                "`{role}` must not be able to dispatch work"
            );
        }
    }
}
