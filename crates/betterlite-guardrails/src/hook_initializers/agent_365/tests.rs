#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::{IgnoredOverride, initialize_agent_365};
use crate::hook_initializers::secret_lookup::SecretLookup;

struct MapLookup(BTreeMap<String, String>);

impl SecretLookup for MapLookup {
    fn get_secret_str(&self, name: &str) -> Option<SecretString> {
        self.0
            .get(name)
            .map(|value| SecretString::from(value.as_str()))
    }
}

fn params(json: &str) -> LitellmParams {
    decode_litellm_params(json.as_bytes()).unwrap()
}

fn lookup(pairs: &[(&str, &str)]) -> MapLookup {
    MapLookup(
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
    )
}

#[test]
fn agent_365_args_copy_the_params_and_the_mode() {
    let secrets = lookup(&[("AGENT365_TENANT_ID", "env-tenant")]);
    let (args, ignored) = initialize_agent_365(
        &params(
            r#"{"guardrail": "agent_365", "mode": "pre_mcp_call", "tenant_id": "tenant",
                "client_id": "client", "client_secret": "secret", "default_on": true,
                "timeout": 2.5, "unreachable_fallback": "fail_open"}"#,
        ),
        Some("agent-check"),
        &secrets,
    )
    .unwrap();
    assert_eq!(args.guardrail_name, "agent-check");
    assert_eq!(args.tenant_id, "tenant");
    assert_eq!(args.client_id, "client");
    assert_eq!(args.client_secret.expose_secret(), "secret");
    assert_eq!(Some(args.request_timeout), Some(2.5_f64));
    assert_eq!(args.unreachable_fallback, UnreachableFallback::FailOpen);
    assert!(args.default_on);
    assert_eq!(
        args.event_hook,
        LitellmParamsMode::One("pre_mcp_call".to_owned())
    );
    assert!(ignored.is_empty());
}

#[test]
fn agent_365_resolves_missing_values_from_the_lookup() {
    let secrets = lookup(&[
        ("AGENT365_TENANT_ID", "env-tenant"),
        ("AGENT365_CLIENT_ID", "env-client"),
        ("AGENT365_CLIENT_SECRET", "env-secret"),
    ]);
    let (args, _) = initialize_agent_365(
        &params(r#"{"guardrail": "agent_365", "mode": "pre_mcp_call", "api_key": "api-secret"}"#),
        Some("agent-check"),
        &secrets,
    )
    .unwrap();
    assert_eq!(args.tenant_id, "env-tenant");
    assert_eq!(args.client_id, "env-client");
    assert_eq!(args.client_secret.expose_secret(), "api-secret");
    let (from_env, _) = initialize_agent_365(
        &params(r#"{"guardrail": "agent_365", "mode": "pre_mcp_call"}"#),
        Some("agent-check"),
        &secrets,
    )
    .unwrap();
    assert_eq!(from_env.client_secret.expose_secret(), "env-secret");
}

#[test]
fn agent_365_prefers_the_params_over_the_lookup_and_api_key() {
    let secrets = lookup(&[
        ("AGENT365_CLIENT_ID", "env-client"),
        ("AGENT365_CLIENT_SECRET", "env-secret"),
    ]);
    let (args, _) = initialize_agent_365(
        &params(
            r#"{"guardrail": "agent_365", "mode": "pre_mcp_call", "tenant_id": "t",
                "client_id": "param-client", "client_secret": "from-param",
                "api_key": "from-key"}"#,
        ),
        Some("name"),
        &secrets,
    )
    .unwrap();
    assert_eq!(args.client_id, "param-client");
    assert_eq!(args.client_secret.expose_secret(), "from-param");
}

#[test]
fn agent_365_treats_an_empty_lookup_value_as_unset() {
    let error = initialize_agent_365(
        &params(r#"{"guardrail": "agent_365", "mode": "pre_mcp_call"}"#),
        Some("name"),
        &lookup(&[("AGENT365_TENANT_ID", "")]),
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Microsoft Agent 365: tenant_id is required"
    );
    let secret = initialize_agent_365(
        &params(
            r#"{"guardrail": "agent_365", "mode": "pre_mcp_call", "tenant_id": "t",
                "client_id": "c"}"#,
        ),
        Some("name"),
        &lookup(&[("AGENT365_CLIENT_SECRET", "")]),
    )
    .unwrap_err();
    assert_eq!(
        secret.to_string(),
        "Microsoft Agent 365: client secret is required. Set client_secret, api_key, or AGENT365_CLIENT_SECRET"
    );
}

#[test]
fn agent_365_treats_an_empty_param_as_unset() {
    let secrets = lookup(&[
        ("AGENT365_TENANT_ID", "env-tenant"),
        ("AGENT365_CLIENT_ID", "env-client"),
        ("AGENT365_CLIENT_SECRET", "env-secret"),
    ]);
    let (args, _) = initialize_agent_365(
        &params(
            r#"{"guardrail": "agent_365", "mode": "pre_mcp_call", "tenant_id": "",
                "client_id": "", "client_secret": ""}"#,
        ),
        Some("agent-check"),
        &secrets,
    )
    .unwrap();
    assert_eq!(args.tenant_id, "env-tenant");
    assert_eq!(args.client_id, "env-client");
    assert_eq!(args.client_secret.expose_secret(), "env-secret");
}

#[test]
fn agent_365_skips_an_empty_api_key_for_the_lookup() {
    let (args, _) = initialize_agent_365(
        &params(
            r#"{"guardrail": "agent_365", "mode": "pre_mcp_call", "tenant_id": "t",
                "client_id": "c", "api_key": ""}"#,
        ),
        Some("agent-check"),
        &lookup(&[("AGENT365_CLIENT_SECRET", "env-secret")]),
    )
    .unwrap();
    assert_eq!(args.client_secret.expose_secret(), "env-secret");
}

#[test]
fn agent_365_defaults_the_request_timeout_only_when_it_is_none() {
    let required = r#""guardrail": "agent_365", "mode": "pre_mcp_call", "tenant_id": "t",
        "client_id": "c", "api_key": "s""#;
    let defaulted = params(&format!("{{{required}}}"));
    let (args, _) = initialize_agent_365(&defaulted, Some("name"), &lookup(&[])).unwrap();
    assert_eq!(Some(args.request_timeout), Some(10.0_f64));
    let explicit = params(&format!("{{{required}, \"timeout\": 0}}"));
    let (zero, _) = initialize_agent_365(&explicit, Some("name"), &lookup(&[])).unwrap();
    assert_eq!(Some(zero.request_timeout), Some(0.0_f64));
}

#[rstest]
#[case::tenant(
    r#"{"guardrail": "agent_365", "mode": "pre_mcp_call"}"#,
    Some("name"),
    "Microsoft Agent 365: tenant_id is required"
)]
#[case::client(
    r#"{"guardrail": "agent_365", "mode": "pre_mcp_call", "tenant_id": "t"}"#,
    Some("name"),
    "Microsoft Agent 365: client_id is required"
)]
#[case::secret(
    r#"{"guardrail": "agent_365", "mode": "pre_mcp_call", "tenant_id": "t", "client_id": "c"}"#,
    Some("name"),
    "Microsoft Agent 365: client secret is required. Set client_secret, api_key, or AGENT365_CLIENT_SECRET"
)]
#[case::credentials_before_name(
    r#"{"guardrail": "agent_365", "mode": "pre_mcp_call"}"#,
    None,
    "Microsoft Agent 365: tenant_id is required"
)]
fn agent_365_refuses_missing_values_in_litellm_order(
    #[case] json: &str,
    #[case] name: Option<&str>,
    #[case] message: &str,
) {
    let error = initialize_agent_365(&params(json), name, &lookup(&[])).unwrap_err();
    assert_eq!(error.to_string(), message);
}

#[test]
fn agent_365_requires_a_non_empty_guardrail_name() {
    let complete = params(
        r#"{"guardrail": "agent_365", "mode": "pre_mcp_call", "tenant_id": "t",
            "client_id": "c", "client_secret": "s"}"#,
    );
    let unnamed = initialize_agent_365(&complete, None, &lookup(&[])).unwrap_err();
    assert_eq!(
        unnamed.to_string(),
        "Microsoft Agent 365: guardrail_name is required"
    );
    let empty = initialize_agent_365(&complete, Some(""), &lookup(&[])).unwrap_err();
    assert_eq!(
        empty.to_string(),
        "Microsoft Agent 365: guardrail_name is required"
    );
}

#[test]
fn agent_365_lists_the_ignored_overrides_in_order() {
    let (_, ignored) = initialize_agent_365(
        &params(
            r#"{"guardrail": "agent_365", "mode": "pre_mcp_call", "tenant_id": "t",
                "client_id": "c", "client_secret": "s", "api_base": "",
                "resource_app_id": "resource", "agent_id": "agent"}"#,
        ),
        Some("name"),
        &lookup(&[]),
    )
    .unwrap();
    assert_eq!(
        ignored,
        vec![
            IgnoredOverride::ApiBase,
            IgnoredOverride::ResourceAppId,
            IgnoredOverride::AgentId
        ]
    );
    assert_eq!(
        ignored
            .iter()
            .map(IgnoredOverride::as_str)
            .collect::<Vec<_>>(),
        ["api_base", "resource_app_id", "agent_id"]
    );
}

#[test]
fn agent_365_skips_a_null_override() {
    let (_, ignored) = initialize_agent_365(
        &params(
            r#"{"guardrail": "agent_365", "mode": "pre_mcp_call", "tenant_id": "t",
                "client_id": "c", "client_secret": "s", "resource_app_id": null,
                "agent_id": null}"#,
        ),
        Some("name"),
        &lookup(&[]),
    )
    .unwrap();
    assert!(ignored.is_empty());
}
