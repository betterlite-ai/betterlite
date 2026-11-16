use crate::responses::error::ResponsesError;
use crate::responses::url::{CompleteUrlParams, complete_url};
use crate::signing::target::RegionInputs;

const US_WEST: &str = "https://bedrock-runtime.us-west-2.amazonaws.com/openai/v1/responses";

// (api_base, runtime endpoint from litellm_params, runtime endpoint from the environment, expected URL)
const OVERRIDES: [(Option<&str>, Option<&str>, Option<&str>, &str); 14] = [
    (None, None, None, US_WEST),
    (
        Some("https://a.example"),
        Some("https://b.example"),
        Some("https://c.example"),
        "https://a.example/openai/v1/responses",
    ),
    (
        Some(""),
        Some("https://b.example"),
        Some("https://c.example"),
        "https://b.example/openai/v1/responses",
    ),
    (
        None,
        Some(""),
        Some("https://c.example"),
        "https://c.example/openai/v1/responses",
    ),
    (None, None, Some(""), US_WEST),
    (
        Some("https://p.example/v1/"),
        None,
        None,
        "https://p.example/openai/v1/responses",
    ),
    (
        Some("https://p.example/openai/v1/responses"),
        None,
        None,
        "https://p.example/openai/v1/responses",
    ),
    (
        Some("https://p.example/v1/responses//"),
        None,
        None,
        "https://p.example/openai/v1/responses",
    ),
    (
        Some("https://p.example/responses"),
        None,
        None,
        "https://p.example/openai/v1/responses",
    ),
    (
        Some("https://p.example/openai/v1"),
        None,
        None,
        "https://p.example/openai/v1/responses",
    ),
    (
        Some("https://p.example/openai/v1/v1"),
        None,
        None,
        "https://p.example/openai/v1/openai/v1/responses",
    ),
    (
        Some("https://p.example/custom?x=1"),
        None,
        None,
        "https://p.example/custom?x=1/openai/v1/responses",
    ),
    (
        Some("https://p.example/v1/responses/v1"),
        None,
        None,
        "https://p.example/v1/responses/openai/v1/responses",
    ),
    (
        Some("https://p.example"),
        None,
        None,
        "https://p.example/openai/v1/responses",
    ),
];

fn url_for(params: CompleteUrlParams<'_>) -> String {
    complete_url(params).expect("the URL builds")
}

pub(super) fn responses_runtime_url() {
    for (api_base, configured, environment, expected) in OVERRIDES {
        let params = CompleteUrlParams {
            api_base,
            aws_bedrock_runtime_endpoint: configured,
            environment_endpoint: environment,
            ..CompleteUrlParams::default()
        };
        assert_eq!(url_for(params), expected);
    }
    region_cases();
}

const ENVIRONMENT: RegionInputs<'static> = RegionInputs {
    environment_region_name: Some("ap-south-1"),
    environment_region: Some("ca-central-1"),
    session_region: Some("sa-east-1"),
};

fn region_cases() {
    let explicit = CompleteUrlParams {
        aws_region_name: Some("eu-west-1"),
        region_inputs: ENVIRONMENT,
        ..CompleteUrlParams::default()
    };
    assert_eq!(
        url_for(explicit),
        "https://bedrock-runtime.eu-west-1.amazonaws.com/openai/v1/responses"
    );
    let from_name = CompleteUrlParams {
        region_inputs: ENVIRONMENT,
        ..CompleteUrlParams::default()
    };
    assert_eq!(
        url_for(from_name),
        "https://bedrock-runtime.ap-south-1.amazonaws.com/openai/v1/responses"
    );
    let from_session = CompleteUrlParams {
        region_inputs: RegionInputs {
            environment_region_name: None,
            environment_region: None,
            ..ENVIRONMENT
        },
        ..CompleteUrlParams::default()
    };
    assert_eq!(
        url_for(from_session),
        "https://bedrock-runtime.sa-east-1.amazonaws.com/openai/v1/responses"
    );
    partition_cases();
}

fn partition_cases() {
    let china = CompleteUrlParams {
        aws_region_name: Some("cn-north-1"),
        ..CompleteUrlParams::default()
    };
    assert_eq!(
        url_for(china),
        "https://bedrock-runtime.cn-north-1.amazonaws.com.cn/openai/v1/responses"
    );
    let invalid = CompleteUrlParams {
        aws_region_name: Some("US-EAST-1"),
        ..CompleteUrlParams::default()
    };
    assert!(matches!(
        complete_url(invalid),
        Err(ResponsesError::Region(_))
    ));
}
