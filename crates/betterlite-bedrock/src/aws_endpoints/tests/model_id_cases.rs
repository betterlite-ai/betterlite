use std::collections::BTreeMap;

use serde_json::value::RawValue;

use crate::aws_endpoints::model_id::{bedrock_model_id, encode_model_id};
use crate::aws_endpoints::provider::BedrockInvokeProvider as Provider;

fn params(source: &str) -> BTreeMap<String, Box<RawValue>> {
    serde_json::from_str(source).unwrap()
}

fn model_id(model: &str, provider: Option<Provider>) -> String {
    let mut optional_params = params("{}");
    bedrock_model_id(&mut optional_params, provider, model).unwrap()
}

pub(super) fn arn_model_path_encoding() {
    assert_eq!(encode_model_id(""), "");
    assert_eq!(
        encode_model_id(
            "arn:aws:bedrock:us-east-1:123456789012:inference-profile/global.anthropic.claude"
        ),
        "arn%3Aaws%3Abedrock%3Aus-east-1%3A123456789012%3Ainference-profile%2Fglobal.anthropic.claude"
    );
    assert_eq!(encode_model_id("ABCxyz019_.-~"), "ABCxyz019_.-~");
    assert_eq!(encode_model_id("a b+c"), "a%20b%2Bc");
    assert_eq!(encode_model_id("%3A"), "%253A");
    assert_eq!(encode_model_id("mod\u{e8}le"), "mod%C3%A8le");
}

pub(super) fn inference_profile_model_id() {
    assert_eq!(
        model_id(
            "bedrock/arn:aws:bedrock:us-east-1:123:inference-profile/global.anthropic.claude",
            None
        ),
        "arn%3Aaws%3Abedrock%3Aus-east-1%3A123%3Ainference-profile%2Fglobal.anthropic.claude"
    );
    assert_eq!(
        model_id(
            "bedrock/converse/arn:aws:bedrock:us-east-1:1:application-inference-profile/abc",
            None
        ),
        "arn%3Aaws%3Abedrock%3Aus-east-1%3A1%3Aapplication-inference-profile%2Fabc"
    );
    assert_eq!(
        model_id("arn:aws:bedrock:us-east-1:1:inference-profile/x", None),
        "arn%3Aaws%3Abedrock%3Aus-east-1%3A1%3Ainference-profile%2Fx"
    );
}

pub(super) fn model_id_encoding_and_mutation() {
    specification_prefixes();
    model_rewrites();
    model_override_mutations();
}

fn specification_prefixes() {
    let unchanged = [
        ("anthropic.claude-3", Provider::Anthropic),
        ("amazon.titan-text-express-v1", Provider::Amazon),
    ];
    for (model, provider) in unchanged {
        assert_eq!(model_id(model, Some(provider)), model, "{model}");
    }
    assert_eq!(model_id("llama/v2/foo", Some(Provider::Llama)), "v2%2Ffoo");
    assert_eq!(
        model_id("deepseek_r1/model", Some(Provider::DeepseekR1)),
        "model"
    );
    assert_eq!(
        model_id("openai.gpt-oss", Some(Provider::Openai)),
        "openai.gpt-oss"
    );
    assert_eq!(
        model_id("qwen2/instruct", Some(Provider::Qwen2)),
        "instruct"
    );
    assert_eq!(model_id("qwen3/coder", Some(Provider::Qwen3)), "coder");
    assert_eq!(
        model_id("stability/sdxl", Some(Provider::Stability)),
        "sdxl"
    );
    assert_eq!(model_id("moonshot/kimi", Some(Provider::Moonshot)), "kimi");
}

fn model_rewrites() {
    assert_eq!(model_id("nova-2/custom", None), "custom");
    assert_eq!(model_id("nova/custom", None), "custom");
    assert_eq!(model_id("bedrock/converse/nova-2/custom", None), "custom");
    assert_eq!(
        model_id("custom/invoke/llama/v2/x", Some(Provider::Llama)),
        "custom%2Fv2%2Fx"
    );
    assert_eq!(
        model_id("invoke/amazon.titan", Some(Provider::Amazon)),
        "amazon.titan"
    );
    assert_eq!(
        model_id(
            "llama/arn:aws:bedrock:us-east-1:1:model/x",
            Some(Provider::Llama)
        ),
        "arn%3Aaws%3Abedrock%3Aus-east-1%3A1%3Amodel%2Fx"
    );
}

fn model_override_mutations() {
    let mut override_params =
        params(r#"{"model_id":"arn:aws:bedrock:us-east-1:1:inference-profile/x","keep":1}"#);
    assert_eq!(
        bedrock_model_id(&mut override_params, None, "ignored").unwrap(),
        "arn%3Aaws%3Abedrock%3Aus-east-1%3A1%3Ainference-profile%2Fx"
    );
    assert_eq!(override_params.len(), 1);
    assert!(override_params.contains_key("keep"));

    let mut empty_override = params(r#"{"model_id":""}"#);
    assert_eq!(
        bedrock_model_id(&mut empty_override, None, "ignored").unwrap(),
        ""
    );
    assert!(empty_override.is_empty());

    let mut encoded_override = params(r#"{"model_id":"%3A"}"#);
    assert_eq!(
        bedrock_model_id(&mut encoded_override, None, "ignored").unwrap(),
        "%253A"
    );

    let mut null_override = params(r#"{"model_id":null,"keep":1}"#);
    assert_eq!(
        bedrock_model_id(&mut null_override, None, "anthropic.claude-3").unwrap(),
        "anthropic.claude-3"
    );
    assert_eq!(null_override.len(), 1);
    assert!(null_override.contains_key("keep"));
}
