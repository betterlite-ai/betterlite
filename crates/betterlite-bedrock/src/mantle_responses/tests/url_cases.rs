use betterlite_models::identity::provider_id::ProviderId;

use crate::mantle_responses::config::{CUSTOM_LLM_PROVIDER, MantleResponsesConfig};
use crate::mantle_responses::error::MantleResponsesError;
use crate::mantle_responses::url::{BASE_SUFFIXES_TO_STRIP, CompleteUrlParams, complete_url};

use super::support;

pub(super) fn mantle_responses_url() {
    constructor_cases();
    default_host_cases();
    configured_base_cases();
    suffix_cases();
    host_cases();
    region_cases();
}

fn complete(
    config: &MantleResponsesConfig,
    api_base: Option<&str>,
    region: Option<&str>,
) -> String {
    let globals = support::globals();
    let params = CompleteUrlParams {
        api_base,
        aws_region_name: region,
    };
    complete_url(config, params, &globals).expect("the url resolves")
}

fn constructor_cases() {
    let default = MantleResponsesConfig::default();
    assert!(default.use_openai_path);
    assert_eq!(CUSTOM_LLM_PROVIDER, ProviderId::BedrockMantle);
    let standard = support::config(false);
    assert!(!standard.use_openai_path);
    assert_eq!(
        BASE_SUFFIXES_TO_STRIP,
        [
            "/openai/v1/responses",
            "/v1/responses",
            "/responses",
            "/openai/v1",
            "/v1"
        ]
    );
}

fn default_host_cases() {
    assert_eq!(
        complete(&support::config(true), None, None),
        "https://bedrock-mantle.us-east-1.api.aws/openai/v1/responses"
    );
    assert_eq!(
        complete(&support::config(false), None, None),
        "https://bedrock-mantle.us-east-1.api.aws/v1/responses"
    );
}

fn configured_base_cases() {
    let mut globals = support::globals();
    globals.mantle_api_base = Some("https://mantle-proxy.example.com/v1".to_owned());
    let params = CompleteUrlParams {
        api_base: None,
        aws_region_name: None,
    };
    assert_eq!(
        complete_url(&support::config(true), params, &globals).expect("globals base"),
        "https://mantle-proxy.example.com/openai/v1/responses"
    );
    let mut overridden = support::globals();
    overridden.mantle_api_base = Some("https://mantle-proxy.example.com".to_owned());
    let explicit = CompleteUrlParams {
        api_base: Some("https://explicit.example.com/v1/"),
        aws_region_name: None,
    };
    assert_eq!(
        complete_url(&support::config(false), explicit, &overridden).expect("explicit base"),
        "https://explicit.example.com/v1/responses"
    );
}

fn suffix_cases() {
    let config = support::config(true);
    let cases = [
        (
            "https://x.example.com/openai/v1/responses",
            "https://x.example.com/openai/v1/responses",
        ),
        (
            "https://x.example.com/v1/responses",
            "https://x.example.com/openai/v1/responses",
        ),
        (
            "https://x.example.com/responses",
            "https://x.example.com/openai/v1/responses",
        ),
        (
            "https://x.example.com/openai/v1",
            "https://x.example.com/openai/v1/responses",
        ),
        (
            "https://x.example.com/v1",
            "https://x.example.com/openai/v1/responses",
        ),
        (
            "https://x.example.com/v1/",
            "https://x.example.com/openai/v1/responses",
        ),
        (
            "https://x.example.com//",
            "https://x.example.com/openai/v1/responses",
        ),
    ];
    for (base, url) in cases {
        assert_eq!(complete(&config, Some(base), None), url, "base {base}");
    }
}

fn host_cases() {
    let config = support::config(true);
    assert_eq!(
        complete(
            &config,
            Some("https://bedrock-mantle.eu-west-1.api.aws/leftover/path"),
            Some("us-west-2")
        ),
        "https://bedrock-mantle.us-west-2.api.aws/openai/v1/responses"
    );
    assert_eq!(
        complete(
            &config,
            Some("HTTPS://BEDROCK-MANTLE.eu-central-1.api.aws/"),
            None
        ),
        "https://bedrock-mantle.eu-central-1.api.aws/openai/v1/responses"
    );
    assert_eq!(
        complete(&config, Some("https://proxy.example.com/mantle"), None),
        "https://proxy.example.com/mantle/openai/v1/responses"
    );
    assert_eq!(
        complete(&config, Some("https://bedrock-mantle.example.com"), None),
        "https://bedrock-mantle.example.com/openai/v1/responses"
    );
    host_edge_cases(&config);
}

fn host_edge_cases(config: &MantleResponsesConfig) {
    for custom in [
        "https://bedrock-mantle..api.aws",
        "https://bedrock-mantle.a.b.api.aws",
        "https://bedrock-mantle.eu-west-1.api.aws.example.com",
    ] {
        assert_eq!(
            complete(config, Some(custom), None),
            format!("{custom}/openai/v1/responses"),
            "base {custom}"
        );
    }
    assert_eq!(
        complete(
            config,
            Some("https://bedrock-mantle.eu-west-1.api.aws\n"),
            None
        ),
        "https://bedrock-mantle.eu-west-1.api.aws/openai/v1/responses"
    );
    assert_eq!(
        complete(
            config,
            Some("https://bedrock-mantle.eu-west-1.api.aws/left\nover"),
            None
        ),
        "https://bedrock-mantle.eu-west-1.api.aws/openai/v1/responses"
    );
}

fn region_cases() {
    let mut globals = support::globals();
    globals.mantle_region = Some("eu-west-3".to_owned());
    let params = CompleteUrlParams {
        api_base: None,
        aws_region_name: None,
    };
    assert_eq!(
        complete_url(&support::config(true), params, &globals).expect("globals region"),
        "https://bedrock-mantle.eu-west-3.api.aws/openai/v1/responses"
    );
    let mut bearer = support::globals();
    bearer.aws_region_name = Some("ap-south-1".to_owned());
    let named = CompleteUrlParams {
        api_base: None,
        aws_region_name: Some("sa-east-1"),
    };
    assert_eq!(
        complete_url(&support::config(true), named, &bearer).expect("explicit region"),
        "https://bedrock-mantle.sa-east-1.api.aws/openai/v1/responses"
    );
    let from_host = CompleteUrlParams {
        api_base: Some("https://bedrock-mantle.us-east-2.api.aws"),
        aws_region_name: None,
    };
    assert_eq!(
        complete_url(&support::config(true), from_host, &bearer).expect("host region"),
        "https://bedrock-mantle.us-east-2.api.aws/openai/v1/responses"
    );
    let invalid = CompleteUrlParams {
        api_base: None,
        aws_region_name: Some("US-EAST-1"),
    };
    let error = complete_url(&support::config(true), invalid, &bearer)
        .expect_err("an unknown region is rejected");
    assert!(matches!(&error, MantleResponsesError::Region(_)));
    assert_eq!(error.status_code(), None);
}
