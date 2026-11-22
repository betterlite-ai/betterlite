#![cfg(test)]

use rstest::rstest;

use super::super::file_timeout_capability;

#[rstest]
#[case::openai(Some("openai"), true)]
#[case::azure(Some("azure"), true)]
#[case::bedrock(Some("bedrock"), true)]
#[case::gemini(Some("gemini"), false)]
#[case::vertex_ai(Some("vertex_ai"), false)]
#[case::manus(Some("manus"), false)]
#[case::anthropic(Some("anthropic"), false)]
#[case::mistral(Some("mistral"), false)]
#[case::xai(Some("xai"), false)]
#[case::hosted_vllm(Some("hosted_vllm"), false)]
#[case::litellm_proxy(Some("litellm_proxy"), false)]
#[case::upper_case_openai(Some("OpenAI"), false)]
#[case::empty(Some(""), false)]
#[case::absent(None, false)]
#[case::unknown(Some("unknown"), false)]
fn only_the_three_listed_providers_keep_the_http_timeout(
    #[case] provider: Option<&str>,
    #[case] expected: bool,
) {
    assert_eq!(file_timeout_capability(provider), expected);
}
