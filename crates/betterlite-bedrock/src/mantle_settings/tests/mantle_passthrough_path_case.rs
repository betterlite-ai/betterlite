#![cfg_attr(
    test,
    expect(
        clippy::cognitive_complexity,
        clippy::too_many_lines,
        reason = "one scenario function per oracle case reads top to bottom as its fixture steps"
    )
)]

use std::collections::BTreeMap;

use secrecy::ExposeSecret as _;

use crate::aws_endpoints::runtime::{BedrockEndpointType, RuntimeEndpoint};
use crate::mantle_inputs::records::MantleGlobals;
use crate::mantle_settings::error::MantleSettingsError;
use crate::mantle_settings::passthrough::{
    CONVERSE_PROVIDER, PassthroughEndpointRequest, passthrough_bearer_token, passthrough_region,
    passthrough_runtime_endpoint,
};

use super::support;

fn globals(pairs: &[(&str, &str)]) -> MantleGlobals {
    support::globals(
        &pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect::<BTreeMap<String, String>>(),
    )
}

fn endpoint(
    api_base: Option<&str>,
    configured: Option<&str>,
    environment: Option<&str>,
    region: &str,
    endpoint_type: BedrockEndpointType,
) -> RuntimeEndpoint {
    passthrough_runtime_endpoint(&PassthroughEndpointRequest {
        api_base,
        configured_endpoint: configured,
        environment_endpoint: environment,
        region,
        endpoint_type,
    })
}

pub(super) fn mantle_passthrough_path() {
    let empty = globals(&[]);
    // The region chain reads the explicit parameter, the configured host, then the environment names and the default.
    assert_eq!(
        passthrough_region(
            &support::object(r#"{"aws_region_name": "eu-west-2"}"#),
            &globals(&[("BEDROCK_MANTLE_REGION", "eu-west-1")]),
        )
        .expect("the explicit region resolves"),
        "eu-west-2"
    );
    assert_eq!(
        passthrough_region(
            &support::object(r#"{"api_base": "https://bedrock-mantle.ap-south-1.api.aws/v1"}"#),
            &empty,
        )
        .expect("the host region resolves"),
        "ap-south-1"
    );
    assert_eq!(
        passthrough_region(
            &support::object("{}"),
            &globals(&[
                (
                    "BEDROCK_MANTLE_API_BASE",
                    "https://bedrock-mantle.us-west-2.api.aws"
                ),
                ("BEDROCK_MANTLE_REGION", "eu-west-1"),
                ("AWS_REGION_NAME", "eu-central-1"),
                ("AWS_REGION", "ap-northeast-1"),
            ]),
        )
        .expect("the environment host resolves"),
        "us-west-2"
    );
    assert_eq!(
        passthrough_region(
            &support::object("{}"),
            &globals(&[
                ("BEDROCK_MANTLE_REGION", "eu-west-1"),
                ("AWS_REGION_NAME", "eu-central-1"),
            ]),
        )
        .expect("the mantle region resolves"),
        "eu-west-1"
    );
    assert_eq!(
        passthrough_region(
            &support::object("{}"),
            &globals(&[
                ("AWS_REGION_NAME", "eu-central-1"),
                ("AWS_REGION", "ap-northeast-1")
            ]),
        )
        .expect("the region name resolves"),
        "eu-central-1"
    );
    assert_eq!(
        passthrough_region(
            &support::object("{}"),
            &globals(&[("AWS_REGION", "ap-northeast-1")])
        )
        .expect("the region resolves"),
        "ap-northeast-1"
    );
    // No model or model_id input reaches the chain, so a region prefix on a model name changes nothing.
    assert_eq!(
        passthrough_region(
            &support::object(r#"{"model": "eu-west-3/openai.gpt-oss-120b"}"#),
            &empty,
        )
        .expect("the model prefix is ignored"),
        "us-east-1"
    );
    let wrong = passthrough_region(&support::object(r#"{"aws_region_name": 7}"#), &empty)
        .expect_err("a non-string region fails");
    assert!(matches!(
        wrong,
        MantleSettingsError::Field {
            field: "aws_region_name"
        }
    ));
    let wrong_base = passthrough_region(
        &support::object(r#"{"api_base": ["https://example"]}"#),
        &empty,
    )
    .expect_err("a non-string base fails");
    assert!(matches!(
        wrong_base,
        MantleSettingsError::Field { field: "api_base" }
    ));
    let invalid = passthrough_region(
        &support::object(r#"{"aws_region_name": "EU-WEST-1"}"#),
        &empty,
    )
    .expect_err("an invalid region fails");
    assert!(matches!(invalid, MantleSettingsError::Region(_)));
    // A recognized Mantle base serves only the OpenAI surface: the request goes to the runtime endpoint built from the same region.
    let mantle = endpoint(
        Some("https://bedrock-mantle.us-west-2.api.aws/v1"),
        None,
        None,
        "us-west-2",
        BedrockEndpointType::Runtime,
    );
    assert_eq!(
        mantle.endpoint_url,
        "https://bedrock-runtime.us-west-2.amazonaws.com"
    );
    assert_eq!(mantle.proxy_endpoint_url, mantle.endpoint_url);
    let trailing = endpoint(
        Some("https://bedrock-mantle.us-west-2.api.aws/"),
        None,
        None,
        "us-west-2",
        BedrockEndpointType::Runtime,
    );
    assert_eq!(
        trailing.endpoint_url,
        "https://bedrock-runtime.us-west-2.amazonaws.com"
    );
    // A configured runtime endpoint is preserved next to a Mantle base, which itself is dropped.
    let configured = endpoint(
        Some("https://bedrock-mantle.eu-west-1.api.aws/v1"),
        Some("https://configured.example"),
        None,
        "eu-west-1",
        BedrockEndpointType::Runtime,
    );
    assert_eq!(configured.endpoint_url, "https://configured.example");
    assert_eq!(configured.proxy_endpoint_url, "https://configured.example");
    // The environment endpoint is used and kept as the proxy endpoint.
    let environment = endpoint(
        Some("https://bedrock-mantle.eu-west-1.api.aws"),
        None,
        Some("https://environment.example"),
        "eu-west-1",
        BedrockEndpointType::Runtime,
    );
    assert_eq!(environment.endpoint_url, "https://environment.example");
    assert_eq!(
        environment.proxy_endpoint_url,
        "https://environment.example"
    );
    let blank_environment = endpoint(
        Some("https://bedrock-mantle.eu-west-1.api.aws"),
        None,
        Some(""),
        "eu-west-1",
        BedrockEndpointType::Runtime,
    );
    assert_eq!(
        blank_environment.endpoint_url,
        "https://bedrock-runtime.eu-west-1.amazonaws.com"
    );
    // A custom base passes through unchanged and wins over the configured endpoint, which stays the proxy endpoint.
    let custom = endpoint(
        Some("https://proxy.example/v1"),
        Some("https://configured.example"),
        None,
        "eu-west-1",
        BedrockEndpointType::Runtime,
    );
    assert_eq!(custom.endpoint_url, "https://proxy.example/v1");
    assert_eq!(custom.proxy_endpoint_url, "https://configured.example");
    // The endpoint type survives next to a Mantle base, and the region keeps its partition suffix.
    let agent = endpoint(
        Some("https://bedrock-mantle.cn-north-1.api.aws"),
        None,
        None,
        "cn-north-1",
        BedrockEndpointType::Agent,
    );
    assert_eq!(
        agent.endpoint_url,
        "https://bedrock-agent-runtime.cn-north-1.amazonaws.com.cn"
    );
    let agentcore = endpoint(
        None,
        None,
        None,
        "us-east-1",
        BedrockEndpointType::Agentcore,
    );
    assert_eq!(
        agentcore.endpoint_url,
        "https://bedrock-agentcore.us-east-1.amazonaws.com"
    );
    // Bearer lookup takes only a string api_key; everything else falls through to the environment names.
    let keyed = passthrough_bearer_token(
        &support::object(r#"{"api_key": "explicit-fixture-key"}"#),
        &globals(&[("BEDROCK_MANTLE_API_KEY", "mantle-fixture-key")]),
    )
    .expect("the explicit key resolves");
    assert_eq!(keyed.expose_secret(), "explicit-fixture-key");
    let empty_key = passthrough_bearer_token(
        &support::object(r#"{"api_key": ""}"#),
        &globals(&[("BEDROCK_MANTLE_API_KEY", "mantle-fixture-key")]),
    )
    .expect("the empty key falls through");
    assert_eq!(empty_key.expose_secret(), "mantle-fixture-key");
    let numeric_key = passthrough_bearer_token(
        &support::object(r#"{"api_key": 42}"#),
        &globals(&[("BEDROCK_MANTLE_API_KEY", "mantle-fixture-key")]),
    )
    .expect("a numeric key falls through");
    assert_eq!(numeric_key.expose_secret(), "mantle-fixture-key");
    let null_key = passthrough_bearer_token(
        &support::object(r#"{"api_key": null}"#),
        &globals(&[("BEDROCK_MANTLE_API_KEY", "mantle-fixture-key")]),
    )
    .expect("a null key falls through");
    assert_eq!(null_key.expose_secret(), "mantle-fixture-key");
    let boolean_key = passthrough_bearer_token(
        &support::object(r#"{"api_key": true}"#),
        &globals(&[("BEDROCK_MANTLE_API_KEY", "mantle-fixture-key")]),
    )
    .expect("a boolean key falls through");
    assert_eq!(boolean_key.expose_secret(), "mantle-fixture-key");
    let list_key = passthrough_bearer_token(
        &support::object(r#"{"api_key": ["fixture"]}"#),
        &globals(&[("BEDROCK_MANTLE_API_KEY", "mantle-fixture-key")]),
    )
    .expect("a list key falls through");
    assert_eq!(list_key.expose_secret(), "mantle-fixture-key");
    let bedrock_token = passthrough_bearer_token(
        &support::object("{}"),
        &globals(&[
            ("BEDROCK_MANTLE_API_KEY", "mantle-fixture-key"),
            ("AWS_BEARER_TOKEN_BEDROCK", "bedrock-fixture-key"),
        ]),
    )
    .expect("the mantle key wins");
    assert_eq!(bedrock_token.expose_secret(), "mantle-fixture-key");
    let standard = passthrough_bearer_token(
        &support::object("{}"),
        &globals(&[("AWS_BEARER_TOKEN_BEDROCK", "bedrock-fixture-key")]),
    )
    .expect("the bedrock token resolves");
    assert_eq!(standard.expose_secret(), "bedrock-fixture-key");
    assert!(passthrough_bearer_token(&support::object("{}"), &empty).is_none());
    assert_eq!(CONVERSE_PROVIDER.to_string(), "bedrock");
}
