use crate::mantle_inputs::chat_endpoint::{
    ChatEndpointInputs, ChatEndpointParams, chat_endpoint_inputs,
};
use crate::mantle_inputs::error::MantleInputError;
use crate::model_routes::{ParamMap, ParamValue};

use super::support::{cost, empty_cost, entry, exposed, globals, secret};

const DEFAULT_BASE: &str = "https://bedrock-mantle.us-east-1.api.aws/v1";

fn resolve(
    params: ChatEndpointParams<'_>,
    model_cost: &ParamMap,
    environment: &[(&str, &str)],
) -> Result<ChatEndpointInputs, MantleInputError> {
    chat_endpoint_inputs(params, &globals(environment), model_cost)
}

fn base(model: Option<&str>, environment: &[(&str, &str)]) -> String {
    resolve(
        ChatEndpointParams {
            model,
            api_base: None,
            api_key: None,
            aws_region_name: None,
        },
        &empty_cost(),
        environment,
    )
    .unwrap()
    .api_base
}

fn rejected(params: ChatEndpointParams<'_>, environment: &[(&str, &str)]) -> bool {
    matches!(
        resolve(params, &empty_cost(), environment),
        Err(MantleInputError::Region(_))
    )
}

fn params<'text>(
    model: Option<&'text str>,
    api_base: Option<&'text str>,
    aws_region_name: Option<&'text str>,
) -> ChatEndpointParams<'text> {
    ChatEndpointParams {
        model,
        api_base,
        api_key: None,
        aws_region_name,
    }
}

pub(super) fn mantle_chat_endpoint_inputs() {
    region_cases();
    api_base_cases();
    explicit_region_cases();
    invalid_region_cases();
    hostname_is_not_a_region_source();
    segment_cases();
    bearer_cases();
}

fn region_cases() {
    assert_eq!(base(Some("openai.gpt-5"), &[]), DEFAULT_BASE);
    assert_eq!(base(None, &[]), DEFAULT_BASE);
    assert_eq!(base(Some(""), &[]), DEFAULT_BASE);
    assert_eq!(
        base(
            Some("openai.gpt-5"),
            &[
                ("BEDROCK_MANTLE_REGION", "ap-south-1"),
                ("AWS_REGION_NAME", "eu-west-1"),
                ("AWS_REGION", "sa-east-1"),
            ]
        ),
        "https://bedrock-mantle.ap-south-1.api.aws/v1"
    );
    assert_eq!(
        base(
            Some("openai.gpt-5"),
            &[
                ("BEDROCK_MANTLE_REGION", ""),
                ("AWS_REGION_NAME", "eu-west-1"),
                ("AWS_REGION", "sa-east-1"),
            ]
        ),
        "https://bedrock-mantle.eu-west-1.api.aws/v1"
    );
    assert_eq!(
        base(
            Some("openai.gpt-5"),
            &[
                ("BEDROCK_MANTLE_REGION", ""),
                ("AWS_REGION_NAME", ""),
                ("AWS_REGION", "sa-east-1"),
            ]
        ),
        "https://bedrock-mantle.sa-east-1.api.aws/v1"
    );
    assert_eq!(
        base(Some("eu-west-1/openai.gpt-5"), &[]),
        "https://bedrock-mantle.eu-west-1.api.aws/v1"
    );
}

fn api_base_cases() {
    let explicit = resolve(
        params(
            Some("openai.gpt-5"),
            Some("https://custom.example/v1/"),
            None,
        ),
        &empty_cost(),
        &[("BEDROCK_MANTLE_API_BASE", "https://ignored.example")],
    )
    .unwrap();
    assert_eq!(explicit.api_base, "https://custom.example/v1/");
    let from_environment = resolve(
        params(Some("openai.gpt-5"), Some(""), None),
        &empty_cost(),
        &[(
            "BEDROCK_MANTLE_API_BASE",
            "https://bedrock-mantle.sa-east-1.api.aws/custom",
        )],
    )
    .unwrap();
    assert_eq!(
        from_environment.api_base,
        "https://bedrock-mantle.sa-east-1.api.aws/custom"
    );
}

fn explicit_region_cases() {
    let explicit = resolve(
        params(Some("eu-west-1/openai.gpt-5"), None, Some("ca-central-1")),
        &empty_cost(),
        &[
            ("BEDROCK_MANTLE_REGION", "ap-south-1"),
            ("AWS_REGION_NAME", "eu-west-1"),
        ],
    )
    .unwrap();
    assert_eq!(
        explicit.api_base,
        "https://bedrock-mantle.ca-central-1.api.aws/v1"
    );
    let empty = resolve(
        params(Some("eu-west-1/openai.gpt-5"), None, Some("")),
        &empty_cost(),
        &[("BEDROCK_MANTLE_REGION", "ap-south-1")],
    )
    .unwrap();
    assert_eq!(
        empty.api_base,
        "https://bedrock-mantle.eu-west-1.api.aws/v1"
    );
}

fn invalid_region_cases() {
    let explicit = rejected(
        params(
            Some("openai.gpt-5"),
            Some("https://custom.example"),
            Some("BAD_REGION"),
        ),
        &[],
    );
    let configured = rejected(
        params(None, Some("https://custom.example"), None),
        &[("BEDROCK_MANTLE_REGION", "BAD_REGION")],
    );
    assert!(explicit);
    assert!(configured);
}

fn hostname_is_not_a_region_source() {
    // the region comes from the configured inputs, never from the supplied hostname
    let host_only = resolve(
        params(None, Some("https://bedrock-mantle.US-EAST-1.api.aws"), None),
        &empty_cost(),
        &[],
    )
    .unwrap();
    assert_eq!(
        host_only.api_base,
        "https://bedrock-mantle.US-EAST-1.api.aws"
    );
}

fn segment_cases() {
    let openai_path = cost(&[(
        "bedrock_mantle/openai.gpt-5",
        entry(&[("use_openai_responses_path", ParamValue::Bool(true))]),
    )]);
    let derived = resolve(
        params(Some("eu-west-1/openai.gpt-5"), None, None),
        &openai_path,
        &[],
    )
    .unwrap();
    assert_eq!(
        derived.api_base,
        "https://bedrock-mantle.eu-west-1.api.aws/openai/v1"
    );
    let unsegmented = resolve(params(Some(""), None, None), &openai_path, &[]).unwrap();
    assert_eq!(unsegmented.api_base, DEFAULT_BASE);
}

fn bearer_cases() {
    let blank = secret("");
    let from_environment = resolve(
        ChatEndpointParams {
            model: None,
            api_base: None,
            api_key: Some(&blank),
            aws_region_name: None,
        },
        &empty_cost(),
        &[
            ("BEDROCK_MANTLE_API_KEY", "mantle-key"),
            ("AWS_BEARER_TOKEN_BEDROCK", "standard-key"),
        ],
    )
    .unwrap();
    assert_eq!(
        exposed(from_environment.api_key.as_ref()),
        Some("mantle-key")
    );
    let explicit = secret("explicit-key");
    let keyed = resolve(
        ChatEndpointParams {
            model: None,
            api_base: None,
            api_key: Some(&explicit),
            aws_region_name: None,
        },
        &empty_cost(),
        &[],
    )
    .unwrap();
    assert_eq!(exposed(keyed.api_key.as_ref()), Some("explicit-key"));
}
