#![cfg(test)]

use std::collections::BTreeSet;

use betterlite_config::sections::mcp_env_values::StdioPolicy;
use betterlite_db::mcp_stored_contracts::{MCPAuth, MCPTransport};
use betterlite_prompts::arguments::json::Json;

use crate::server_payload::RequestError;
use crate::server_payload::request::{
    NewMCPServerRequest, UpdateMCPServerRequest, is_per_server_oauth_discovery_eligible,
};
use crate::server_record::OAuth2Flow;

const DISABLED: &str = "stdio MCP servers are disabled on this proxy. Set LITELLM_ENABLE_MCP_STDIO=true on the proxy and restart to enable them";
const DISCOVERY: &str = "per_server_oauth_discovery is only supported for auth_type oauth2 with oauth2_flow authorization_code and without delegate_auth_to_upstream.";
const ALLOWED: &str = "['deno', 'docker', 'node', 'npx', 'python', 'python3', 'uvx']";

fn payload(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn policy(enabled: bool) -> StdioPolicy {
    StdioPolicy {
        enabled,
        allowed_commands: [
            "deno".to_owned(),
            "docker".to_owned(),
            "node".to_owned(),
            "npx".to_owned(),
            "python".to_owned(),
            "python3".to_owned(),
            "uvx".to_owned(),
        ]
        .into_iter()
        .collect::<BTreeSet<String>>(),
    }
}

fn new_request(source: &str, stdio: &StdioPolicy) -> Result<NewMCPServerRequest, RequestError> {
    NewMCPServerRequest::from_value(&payload(source), stdio)
}

fn update_request(
    source: &str,
    stdio: &StdioPolicy,
) -> Result<UpdateMCPServerRequest, RequestError> {
    UpdateMCPServerRequest::from_value(&payload(source), stdio)
}

fn value_error<T>(result: Result<T, RequestError>) -> String {
    match result {
        Err(RequestError::Value(text)) => text,
        Err(RequestError::Shape(error)) => panic!("expected a validator error, got {error}"),
        Ok(_) => panic!("expected an error"),
    }
}

#[test]
fn defaults_match_the_litellm_model() {
    let request = new_request(r#"{"transport":"http","url":"u"}"#, &policy(true)).unwrap();
    assert_eq!(request.transport, MCPTransport::Http);
    assert!(request.args.is_empty());
    assert!(request.env.is_empty());
    assert!(request.mcp_access_groups.is_empty());
    assert!(request.byok_description.is_empty());
    assert_eq!(request.auth_type, None);
    assert!(request.credentials.is_none());
    assert_eq!(request.dcr_bridge, None);
    assert_eq!(request.oauth2_flow, None);
    assert!(!request.allow_all_keys);
    assert!(request.available_on_public_internet);
    assert!(!request.delegate_auth_to_upstream);
    assert!(!request.oauth_passthrough);
    assert!(!request.per_server_oauth_discovery);
    assert!(!request.is_byok);
}

#[test]
fn a_missing_transport_skips_the_url_check_and_defaults_to_sse() {
    let request = new_request("{}", &policy(true)).unwrap();
    assert_eq!(request.transport, MCPTransport::Sse);
}

#[test]
fn http_needs_a_url_or_a_spec_path() {
    assert_eq!(
        value_error(new_request(r#"{"transport":"http"}"#, &policy(true))),
        "url or spec_path is required for HTTP/SSE transport"
    );
    assert_eq!(
        value_error(new_request(r#"{"transport":"sse"}"#, &policy(true))),
        "url or spec_path is required for HTTP/SSE transport"
    );
}

#[test]
fn sse_accepts_a_spec_path() {
    let request = new_request(
        r#"{"transport":"sse","spec_path":"spec.json"}"#,
        &policy(true),
    );
    assert!(request.is_ok());
}

#[test]
fn an_empty_or_null_url_is_not_a_url() {
    assert_eq!(
        value_error(new_request(
            r#"{"transport":"sse","url":""}"#,
            &policy(true)
        )),
        "url or spec_path is required for HTTP/SSE transport"
    );
    assert_eq!(
        value_error(new_request(
            r#"{"transport":"sse","url":null}"#,
            &policy(true)
        )),
        "url or spec_path is required for HTTP/SSE transport"
    );
}

#[test]
fn a_non_string_transport_is_a_shape_error() {
    assert!(matches!(
        new_request(r#"{"transport":null}"#, &policy(true)),
        Err(RequestError::Shape(_))
    ));
    assert!(matches!(
        new_request(r#"{"transport":5}"#, &policy(true)),
        Err(RequestError::Shape(_))
    ));
}

#[test]
fn stdio_is_blocked_when_the_policy_is_off() {
    assert_eq!(
        value_error(new_request(r#"{"transport":"stdio"}"#, &policy(false))),
        DISABLED
    );
}

#[test]
fn stdio_needs_a_command_before_args() {
    assert_eq!(
        value_error(new_request(r#"{"transport":"stdio"}"#, &policy(true))),
        "command is required for stdio transport"
    );
    assert_eq!(
        value_error(new_request(
            r#"{"transport":"stdio","command":"npx"}"#,
            &policy(true)
        )),
        "args is required for stdio transport"
    );
    assert_eq!(
        value_error(new_request(
            r#"{"transport":"stdio","command":"npx","args":[]}"#,
            &policy(true)
        )),
        "args is required for stdio transport"
    );
}

#[test]
fn stdio_checks_the_basename_of_the_command() {
    assert!(
        new_request(
            r#"{"transport":"stdio","command":"/usr/bin/node","args":["x"]}"#,
            &policy(true)
        )
        .is_ok()
    );
    assert_eq!(
        value_error(new_request(
            r#"{"transport":"stdio","command":"npx.exe","args":["x"]}"#,
            &policy(true)
        )),
        format!(
            "Command 'npx.exe' is not in the allowed commands list for stdio transport. Allowed commands: {ALLOWED}"
        )
    );
}

#[test]
fn discovery_runs_before_the_transport_check() {
    assert_eq!(
        value_error(new_request(
            r#"{"transport":"stdio","per_server_oauth_discovery":true}"#,
            &policy(false)
        )),
        DISCOVERY
    );
}

#[test]
fn dcr_bridge_runs_before_the_transport_check() {
    assert_eq!(
        value_error(new_request(
            r#"{"transport":"stdio","dcr_bridge":true}"#,
            &policy(false)
        )),
        "dcr_bridge is only supported for auth_type true_passthrough or oauth_delegate (got None). The DCR bridge serves gateway-hosted OAuth discovery for the client-forwarded token modes; interactive oauth2 servers already run the gateway authorization-code flow."
    );
}

#[test]
fn discovery_needs_oauth2_authorization_code_without_delegation() {
    let base = r#"{"transport":"http","url":"u","per_server_oauth_discovery":true"#;
    assert!(
        new_request(
            &format!(r#"{base},"auth_type":"oauth2","oauth2_flow":"authorization_code"}}"#),
            &policy(true)
        )
        .is_ok()
    );
    assert_eq!(
        value_error(new_request(
            &format!(r#"{base},"auth_type":"oauth2","oauth2_flow":"client_credentials"}}"#),
            &policy(true)
        )),
        DISCOVERY
    );
    assert_eq!(
        value_error(new_request(
            &format!(
                r#"{base},"auth_type":"oauth2","oauth2_flow":"authorization_code","delegate_auth_to_upstream":true}}"#
            ),
            &policy(true)
        )),
        DISCOVERY
    );
}

#[test]
fn a_new_payload_must_carry_both_eligibility_keys() {
    let base = r#"{"transport":"http","url":"u","per_server_oauth_discovery":true"#;
    assert_eq!(
        value_error(new_request(
            &format!(r#"{base},"auth_type":"oauth2"}}"#),
            &policy(true)
        )),
        DISCOVERY
    );
    assert_eq!(
        value_error(new_request(
            &format!(r#"{base},"oauth2_flow":"authorization_code"}}"#),
            &policy(true)
        )),
        DISCOVERY
    );
}

#[test]
fn update_tolerates_missing_eligibility_keys_but_not_null_ones() {
    let base = r#"{"server_id":"s","transport":"http","url":"u","per_server_oauth_discovery":true"#;
    assert!(update_request(&format!("{base}}}"), &policy(true)).is_ok());
    assert!(
        update_request(
            &format!(r#"{base},"auth_type":"oauth2","oauth2_flow":"authorization_code"}}"#),
            &policy(true)
        )
        .is_ok()
    );
    assert_eq!(
        value_error(update_request(
            &format!(r#"{base},"auth_type":null}}"#),
            &policy(true)
        )),
        DISCOVERY
    );
    assert_eq!(
        value_error(update_request(
            &format!(r#"{base},"oauth2_flow":null}}"#),
            &policy(true)
        )),
        DISCOVERY
    );
}

#[test]
fn dcr_bridge_needs_a_forwarded_token_auth_type() {
    let base = r#"{"transport":"http","url":"u","dcr_bridge":true"#;
    assert_eq!(
        value_error(new_request(&format!("{base}}}"), &policy(true))),
        "dcr_bridge is only supported for auth_type true_passthrough or oauth_delegate (got None). The DCR bridge serves gateway-hosted OAuth discovery for the client-forwarded token modes; interactive oauth2 servers already run the gateway authorization-code flow."
    );
    assert!(
        new_request(
            &format!(r#"{base},"auth_type":"true_passthrough"}}"#),
            &policy(true)
        )
        .is_ok()
    );
    assert!(
        new_request(
            &format!(r#"{base},"auth_type":"oauth_delegate"}}"#),
            &policy(true)
        )
        .is_ok()
    );
    assert_eq!(
        value_error(new_request(
            &format!(r#"{base},"auth_type":"oauth2"}}"#),
            &policy(true)
        )),
        "dcr_bridge is only supported for auth_type true_passthrough or oauth_delegate (got 'oauth2'). The DCR bridge serves gateway-hosted OAuth discovery for the client-forwarded token modes; interactive oauth2 servers already run the gateway authorization-code flow."
    );
    assert_eq!(
        value_error(new_request(
            &format!(r#"{base},"auth_type":null}}"#),
            &policy(true)
        )),
        "dcr_bridge is only supported for auth_type true_passthrough or oauth_delegate (got None). The DCR bridge serves gateway-hosted OAuth discovery for the client-forwarded token modes; interactive oauth2 servers already run the gateway authorization-code flow."
    );
}

#[test]
fn update_dcr_bridge_skips_an_absent_auth_type() {
    assert!(
        update_request(
            r#"{"server_id":"s","transport":"http","url":"u","dcr_bridge":true}"#,
            &policy(true)
        )
        .is_ok()
    );
    assert_eq!(
        value_error(update_request(
            r#"{"server_id":"s","transport":"http","url":"u","dcr_bridge":true,"auth_type":null}"#,
            &policy(true)
        )),
        "dcr_bridge is only supported for auth_type true_passthrough or oauth_delegate (got None). The DCR bridge serves gateway-hosted OAuth discovery for the client-forwarded token modes; interactive oauth2 servers already run the gateway authorization-code flow."
    );
}

#[test]
fn update_requires_a_server_id() {
    assert!(matches!(
        update_request(r#"{"transport":"http","url":"u"}"#, &policy(true)),
        Err(RequestError::Shape(_))
    ));
}

#[test]
fn a_non_object_payload_is_a_shape_error() {
    assert!(matches!(
        new_request("[]", &policy(true)),
        Err(RequestError::Shape(_))
    ));
    assert!(matches!(
        new_request(r#""text""#, &policy(true)),
        Err(RequestError::Shape(_))
    ));
}

#[test]
fn auth_type_and_flow_parse_into_their_enums() {
    let request = new_request(
        r#"{"transport":"http","url":"u","auth_type":"true_passthrough","oauth2_flow":"client_credentials"}"#,
        &policy(true),
    )
    .unwrap();
    assert_eq!(request.auth_type, Some(MCPAuth::TruePassthrough));
    assert_eq!(request.oauth2_flow, Some(OAuth2Flow::ClientCredentials));
}

#[test]
fn an_unknown_auth_type_is_a_shape_error() {
    assert!(matches!(
        new_request(
            r#"{"transport":"http","url":"u","auth_type":"magic"}"#,
            &policy(true)
        ),
        Err(RequestError::Shape(_))
    ));
    assert!(matches!(
        new_request(
            r#"{"transport":"http","url":"u","oauth2_flow":"bad"}"#,
            &policy(true)
        ),
        Err(RequestError::Shape(_))
    ));
}

#[test]
fn ordered_maps_keep_key_order_and_replace_a_repeated_key_in_place() {
    let request = new_request(
        r#"{"transport":"http","url":"u","static_headers":{"z":"1","a":"2","z":"3"}}"#,
        &policy(true),
    )
    .unwrap();
    let headers = request.static_headers.unwrap();
    let keys: Vec<&str> = headers.keys().map(String::as_str).collect();
    assert_eq!(keys, ["z", "a"]);
    assert_eq!(headers.get("z").map(String::as_str), Some("3"));
}

#[test]
fn debug_prints_only_the_safe_fields() {
    let request = new_request(
        r#"{"transport":"http","url":"https://secret.example","server_name":"srv","auth_type":"bearer_token","credentials":{"client_secret":"hidden"}}"#,
        &policy(true),
    )
    .unwrap();
    let debug = format!("{request:?}");
    assert!(debug.contains("srv"));
    assert!(debug.contains("Http"));
    assert!(debug.contains("BearerToken"));
    assert!(!debug.contains("secret.example"));
    assert!(!debug.contains("hidden"));
}

#[test]
fn eligibility_needs_every_condition() {
    assert!(is_per_server_oauth_discovery_eligible(
        Some(MCPAuth::Oauth2),
        Some(OAuth2Flow::AuthorizationCode),
        false
    ));
    assert!(!is_per_server_oauth_discovery_eligible(
        Some(MCPAuth::Oauth2),
        Some(OAuth2Flow::ClientCredentials),
        false
    ));
    assert!(!is_per_server_oauth_discovery_eligible(
        Some(MCPAuth::Oauth2),
        Some(OAuth2Flow::AuthorizationCode),
        true
    ));
    assert!(!is_per_server_oauth_discovery_eligible(
        None,
        Some(OAuth2Flow::AuthorizationCode),
        false
    ));
    assert!(!is_per_server_oauth_discovery_eligible(
        Some(MCPAuth::Oauth2),
        None,
        false
    ));
}
