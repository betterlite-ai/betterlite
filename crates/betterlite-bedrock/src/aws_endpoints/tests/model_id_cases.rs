use std::collections::BTreeMap;

use serde_json::value::RawValue;

use crate::aws_endpoints::model_id::bedrock_model_id;
use crate::aws_endpoints::provider::BedrockInvokeProvider as Provider;

const ARN: &str =
    "arn:aws:bedrock:us-east-1:123456789012:inference-profile/global.anthropic.claude";
const ENCODED_ARN: &str =
    "arn%3Aaws%3Abedrock%3Aus-east-1%3A123456789012%3Ainference-profile%2Fglobal.anthropic.claude";

fn params(source: &str) -> BTreeMap<String, Box<RawValue>> {
    serde_json::from_str(source).unwrap()
}

fn model_id(model: &str, provider: Option<Provider>) -> String {
    let mut optional_params = params("{}");
    bedrock_model_id(&mut optional_params, provider, model).unwrap()
}

fn encoded_override(model_id: &str) -> String {
    let quoted = serde_json::to_string(model_id).unwrap();
    let mut optional_params = params(&format!(r#"{{"model_id":{quoted}}}"#));
    bedrock_model_id(&mut optional_params, None, "ignored").unwrap()
}

fn encoded_model(model: &str) -> String {
    let mut optional_params = params("{}");
    bedrock_model_id(&mut optional_params, None, model).unwrap()
}

pub(super) fn arn_model_path_encoding() {
    assert_eq!(encoded_override(""), "");
    assert_eq!(encoded_override(ARN), ENCODED_ARN);
    assert_eq!(encoded_override("ABCxyz019_.-~"), "ABCxyz019_.-~");
    assert_eq!(encoded_override("a b+c"), "a%20b%2Bc");
    assert_eq!(encoded_override("%3A"), "%253A");
    assert_eq!(encoded_override("mod\u{e8}le"), "mod%C3%A8le");
    assert_eq!(encoded_model(ARN), ENCODED_ARN);
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
    routing_rewrites();
    precedence_rewrites();
    model_override_mutations();
}

fn specification_prefixes() {
    let unchanged = [
        ("anthropic.claude-3", Provider::Anthropic),
        ("amazon.titan-text-express-v1", Provider::Amazon),
        ("meta.llama3-8b:0", Provider::Llama),
        ("deepseek.r1:0", Provider::DeepseekR1),
        ("openai.gpt-oss:1", Provider::Openai),
        ("qwen2.instruct:0", Provider::Qwen2),
        ("qwen3.coder:0", Provider::Qwen3),
        ("stability.sdxl:0", Provider::Stability),
        ("moonshot.kimi:0", Provider::Moonshot),
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

fn routing_rewrites() {
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

fn precedence_rewrites() {
    let cases = [
        ("x/nova/y", None, "x%2Fy"),
        ("x/nova-2/y", None, "x%2Fy"),
        ("custom/openai/x", Some(Provider::Openai), "custom%2Fx"),
        ("a/llama/b/llama/c", Some(Provider::Llama), "a%2Fb%2Fc"),
        ("a/invoke/b/invoke/c", None, "a/b/invoke/c"),
        ("a/nova/llama/x", Some(Provider::Llama), "a%2Fnova%2Fx"),
    ];
    for (model, provider, expected) in cases {
        assert_eq!(model_id(model, provider), expected, "{model}");
    }
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
