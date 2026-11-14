#![cfg(test)]

use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use rstest::rstest;
use secrecy::SecretString;

use crate::hook_initializers::agent_365::initialize_agent_365;
use crate::hook_initializers::azure::initialize_azure;
use crate::hook_initializers::ibm::initialize_ibm;
use crate::hook_initializers::model_armor::initialize_model_armor;
use crate::hook_initializers::openai::initialize_openai;
use crate::hook_initializers::purview::initialize_purview;
use crate::hook_initializers::secret_lookup::SecretLookup;

const SECRET: &str = "sk-guardrail-secret-value";

struct NoSecrets;

impl SecretLookup for NoSecrets {
    fn get_secret_str(&self, _name: &str) -> Option<SecretString> {
        None
    }
}

fn params(json: &str) -> LitellmParams {
    decode_litellm_params(json.as_bytes()).unwrap()
}

fn agent_error(json: &str, name: Option<&str>) -> String {
    initialize_agent_365(&params(json), name, &NoSecrets)
        .unwrap_err()
        .to_string()
}

fn azure_error(json: &str, name: Option<&str>) -> String {
    initialize_azure(&params(json), name)
        .unwrap_err()
        .to_string()
}

fn ibm_error(json: &str, name: Option<&str>) -> String {
    initialize_ibm(&params(json), name).unwrap_err().to_string()
}

fn purview_error(json: &str, name: Option<&str>) -> String {
    initialize_purview(&params(json), name)
        .unwrap_err()
        .to_string()
}

fn openai_error(json: &str, name: Option<&str>) -> String {
    initialize_openai(&params(json), name)
        .unwrap_err()
        .to_string()
}

#[derive(Clone, Copy)]
enum Vendor {
    Agent,
    Azure,
    Ibm,
    Purview,
    OpenAi,
}

fn error_for(vendor: Vendor, json: &str, name: Option<&str>) -> String {
    match vendor {
        Vendor::Agent => agent_error(json, name),
        Vendor::Azure => azure_error(json, name),
        Vendor::Ibm => ibm_error(json, name),
        Vendor::Purview => purview_error(json, name),
        Vendor::OpenAi => openai_error(json, name),
    }
}

const AGENT_ONLY_SECRET: &str = r#"{"guardrail": "agent_365", "mode": "pre_call", "client_secret": "sk-guardrail-secret-value"}"#;
const AZURE_NO_BASE: &str = r#"{"guardrail": "azure/prompt_shield", "mode": "pre_call", "api_key": "sk-guardrail-secret-value"}"#;
const IBM_NO_BASE_URL: &str = r#"{"guardrail": "ibm_guardrails", "mode": "pre_call", "auth_token": "sk-guardrail-secret-value"}"#;
const PURVIEW_NO_NAME: &str = r#"{"guardrail": "microsoft_purview", "mode": "pre_call", "tenant_id": "tenant", "client_id": "client", "api_key": "sk-guardrail-secret-value"}"#;
const OPENAI_NO_NAME: &str = r#"{"guardrail": "openai_moderation", "mode": "during_call", "api_key": "sk-guardrail-secret-value"}"#;

#[rstest]
#[case::agent(
    Vendor::Agent,
    AGENT_ONLY_SECRET,
    Some("name"),
    "Microsoft Agent 365: tenant_id is required"
)]
#[case::azure(
    Vendor::Azure,
    AZURE_NO_BASE,
    Some("name"),
    "Azure Content Safety: api_base is required"
)]
#[case::ibm(
    Vendor::Ibm,
    IBM_NO_BASE_URL,
    Some("name"),
    "IBM Guardrails: base_url is required"
)]
#[case::purview(
    Vendor::Purview,
    PURVIEW_NO_NAME,
    None,
    "Microsoft Purview: guardrail_name is required"
)]
#[case::openai(
    Vendor::OpenAi,
    OPENAI_NO_NAME,
    None,
    "OpenAI Moderation: guardrail_name is required"
)]
fn required_errors_carry_no_parameter_value(
    #[case] vendor: Vendor,
    #[case] json: &str,
    #[case] name: Option<&str>,
    #[case] expected: &str,
) {
    let message = error_for(vendor, json, name);
    assert_eq!(message, expected);
    assert!(!message.contains(SECRET));
}

#[test]
fn the_special_secret_messages_match_litellm() {
    let agent = agent_error(
        r#"{"guardrail": "agent_365", "mode": "pre_call", "tenant_id": "tenant", "client_id": "client"}"#,
        Some("name"),
    );
    assert_eq!(
        agent,
        "Microsoft Agent 365: client secret is required. Set client_secret, api_key, or AGENT365_CLIENT_SECRET"
    );
    let purview = purview_error(
        r#"{"guardrail": "microsoft_purview", "mode": "pre_call", "tenant_id": "tenant", "client_id": "client"}"#,
        Some("name"),
    );
    assert_eq!(
        purview,
        "Microsoft Purview: client_secret (or api_key) is required"
    );
}

const AGENT_DEBUG: &str = r#"{"guardrail": "agent_365", "mode": "pre_call", "tenant_id": "tenant", "client_id": "client", "client_secret": "sk-guardrail-secret-value"}"#;
const AZURE_DEBUG: &str = r#"{"guardrail": "azure/prompt_shield", "mode": "pre_call", "api_key": "sk-guardrail-secret-value", "api_base": "https://azure.example"}"#;
const IBM_DEBUG: &str = r#"{"guardrail": "ibm_guardrails", "mode": "pre_call", "auth_token": "sk-guardrail-secret-value", "base_url": "https://ibm.example", "detector_id": "jailbreak"}"#;
const PURVIEW_DEBUG: &str = r#"{"guardrail": "microsoft_purview", "mode": "pre_call", "tenant_id": "tenant", "client_id": "client", "client_secret": "sk-guardrail-secret-value"}"#;
const ARMOR_DEBUG: &str = r#"{"guardrail": "model_armor", "mode": "pre_call", "credentials": "sk-guardrail-secret-value"}"#;
const OPENAI_DEBUG: &str = r#"{"guardrail": "openai_moderation", "mode": "during_call", "api_key": "sk-guardrail-secret-value"}"#;

fn agent_debug() -> String {
    let (args, _) = initialize_agent_365(&params(AGENT_DEBUG), Some("name"), &NoSecrets).unwrap();
    format!("{args:?}")
}

fn azure_debug() -> String {
    let args = initialize_azure(&params(AZURE_DEBUG), Some("name")).unwrap();
    format!("{args:?}")
}

fn ibm_debug() -> String {
    let args = initialize_ibm(&params(IBM_DEBUG), Some("name")).unwrap();
    format!("{args:?}")
}

fn purview_debug() -> String {
    let args = initialize_purview(&params(PURVIEW_DEBUG), Some("name")).unwrap();
    format!("{args:?}")
}

fn armor_debug() -> String {
    let args = initialize_model_armor(&params(ARMOR_DEBUG), Some("name"));
    format!("{args:?}")
}

fn openai_debug() -> String {
    let args = initialize_openai(&params(OPENAI_DEBUG), Some("name")).unwrap();
    format!("{args:?}")
}

#[derive(Clone, Copy)]
enum Dump {
    Agent,
    Azure,
    Ibm,
    Purview,
    Armor,
    OpenAi,
}

fn dump_for(kind: Dump) -> String {
    match kind {
        Dump::Agent => agent_debug(),
        Dump::Azure => azure_debug(),
        Dump::Ibm => ibm_debug(),
        Dump::Purview => purview_debug(),
        Dump::Armor => armor_debug(),
        Dump::OpenAi => openai_debug(),
    }
}

#[rstest]
#[case::agent(Dump::Agent)]
#[case::azure(Dump::Azure)]
#[case::ibm(Dump::Ibm)]
#[case::purview(Dump::Purview)]
#[case::armor(Dump::Armor)]
#[case::openai(Dump::OpenAi)]
fn debug_output_of_every_args_struct_redacts_the_secret(#[case] kind: Dump) {
    assert!(!dump_for(kind).contains(SECRET));
}
