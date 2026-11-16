#![cfg(test)]

use super::{
    BLOCK_DECISIONS, BLOCKED_TURN_MEMORY, BLOCKED_TURN_TTL_SECONDS, CLIENT_BY_USER_AGENT,
    CREDENTIAL_FIELDS, DERIVED_SESSION_PREFIX, DETECT_PATH, GATEWAY_NAME, IDENTITY_METADATA_KEYS,
    KEY_PREFIX, PLACEHOLDER_IDENTITIES, PREAMBLE_ROLES, PROVIDER_BODY_KEYS, REDACTED_KEYS,
    REDACTED_VALUE, RESPONSE_PHASE, UNDECIDED_DECISIONS,
};

#[test]
fn pins_the_v3_wire_contract() {
    assert_eq!(DETECT_PATH, "/api/v3/detect");
    assert_eq!(KEY_PREFIX, "sk_agt_");
    assert_eq!(RESPONSE_PHASE, "response-sync");
    assert_eq!(DERIVED_SESSION_PREFIX, "litellm-");
    assert_eq!(GATEWAY_NAME, "LiteLLM");
    assert_eq!(BLOCKED_TURN_MEMORY, 10_000);
    assert_eq!(BLOCKED_TURN_TTL_SECONDS, 86_400);
    assert_eq!(REDACTED_VALUE, "[redacted]");
}

#[test]
fn pins_the_v3_decision_sets() {
    assert_eq!(BLOCK_DECISIONS, ["block", "deny"]);
    assert_eq!(UNDECIDED_DECISIONS, ["ask"]);
    assert_eq!(PREAMBLE_ROLES, ["system", "developer"]);
    assert_eq!(REDACTED_KEYS, ["tools", "mcp_servers"]);
    assert_eq!(
        CREDENTIAL_FIELDS,
        ["authorization_token", "authorization", "headers"]
    );
    assert_eq!(
        PLACEHOLDER_IDENTITIES,
        ["default_user_id", "litellm_proxy_master_key"]
    );
    assert_eq!(CLIENT_BY_USER_AGENT, [("claude-cli/", "claude", "Claude")]);
}

#[test]
fn pins_the_v3_identity_and_body_lists() {
    assert_eq!(
        IDENTITY_METADATA_KEYS,
        [
            "user_api_key_end_user_id",
            "user_api_key_user_email",
            "user_api_key_user_id",
            "user_api_key_alias",
            "user_api_key_team_id",
        ]
    );
    assert_eq!(PROVIDER_BODY_KEYS.len(), 53);
    assert!(PROVIDER_BODY_KEYS.contains(&"model"));
    assert!(PROVIDER_BODY_KEYS.contains(&"messages"));
    assert!(PROVIDER_BODY_KEYS.contains(&"tools"));
    assert!(PROVIDER_BODY_KEYS.contains(&"mcp_servers"));
    assert!(PROVIDER_BODY_KEYS.contains(&"input"));
    assert!(PROVIDER_BODY_KEYS.contains(&"instructions"));
    assert!(PROVIDER_BODY_KEYS.contains(&"session_id"));
}
