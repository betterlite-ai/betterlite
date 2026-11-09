use crate::count_tokens::endpoint::{CountTokensEndpoint, count_tokens_endpoint};

use super::support::MODEL;

fn endpoint(
    model: &str,
    region: &str,
    api_base: Option<&str>,
    runtime: Option<&str>,
    environment: Option<&str>,
) -> String {
    count_tokens_endpoint(CountTokensEndpoint {
        model,
        aws_region_name: region,
        api_base,
        aws_bedrock_runtime_endpoint: runtime,
        environment_endpoint: environment,
    })
}

pub(super) fn count_endpoint_path() {
    assert_eq!(
        endpoint(
            "bedrock/converse/us.anthropic.claude-3-5-sonnet-20241022-v2:0",
            "eu-west-1",
            Some("https://api.example"),
            Some("https://runtime.example"),
            Some("https://env.example"),
        ),
        "https://api.example/model/anthropic.claude-3-5-sonnet-20241022-v2%3A0/count-tokens"
    );
    assert_eq!(
        endpoint(
            "bedrock/us.anthropic.claude-3-5-sonnet-20241022-v2:0",
            "eu-west-1",
            None,
            Some("https://runtime.example"),
            Some("https://env.example"),
        ),
        "https://runtime.example/model/anthropic.claude-3-5-sonnet-20241022-v2%3A0/count-tokens"
    );
    assert_eq!(
        endpoint(MODEL, "eu-west-1", None, None, Some("https://env.example")),
        "https://env.example/model/anthropic.claude-3-5-sonnet-20241022-v2%3A0/count-tokens"
    );
    regional_defaults();
    partition_and_encoding();
}

fn regional_defaults() {
    assert_eq!(
        endpoint(MODEL, "eu-west-1", None, None, None),
        "https://bedrock-runtime.eu-west-1.amazonaws.com/model/anthropic.claude-3-5-sonnet-20241022-v2%3A0/count-tokens"
    );
    assert_eq!(
        endpoint(MODEL, "cn-north-1", None, None, None),
        "https://bedrock-runtime.cn-north-1.amazonaws.com.cn/model/anthropic.claude-3-5-sonnet-20241022-v2%3A0/count-tokens"
    );
}

fn partition_and_encoding() {
    assert_eq!(
        endpoint(
            MODEL,
            "eu-west-1",
            Some(""),
            Some("https://runtime.example"),
            Some("https://env.example"),
        ),
        "/model/anthropic.claude-3-5-sonnet-20241022-v2%3A0/count-tokens"
    );
    assert_eq!(
        endpoint(
            MODEL,
            "eu-west-1",
            None,
            Some(""),
            Some("https://env.example")
        ),
        "/model/anthropic.claude-3-5-sonnet-20241022-v2%3A0/count-tokens"
    );
    assert_eq!(
        endpoint(
            "bedrock/mantle/anthropic.claude-sonnet-5",
            "us-east-1",
            None,
            None,
            Some(""),
        ),
        "https://bedrock-runtime.us-east-1.amazonaws.com/model/anthropic.claude-sonnet-5/count-tokens"
    );
    assert_eq!(
        endpoint(
            "arn:aws:bedrock:us-east-1:123456789012:inference-profile/global.anthropic.claude",
            "us-east-1",
            None,
            None,
            None,
        ),
        "https://bedrock-runtime.us-east-1.amazonaws.com/model/anthropic.claude/count-tokens"
    );
}
