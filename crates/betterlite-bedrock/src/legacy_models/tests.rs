#![cfg(test)]

use betterlite_config::sections::bedrock_legacy_models::families::{
    Ai21Values, AnthropicValues, CohereValues, LlamaValues, MistralValues, TitanValues,
};
use betterlite_models::decisions::Json as ConfigJson;
use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;

use crate::legacy_models::constructors::apply;
use crate::legacy_models::family::{AttributeKind, ClassAttribute, Family, LegacyConfig};
use crate::legacy_models::mapping::{
    MappingRequest, map_and_modify_arg, map_openai_params, supported_openai_params,
};
use crate::legacy_models::mistral::{MistralOutputError, output_text};

fn object(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

fn projected(class: &LegacyConfig) -> Object {
    let text = serde_json::to_string(&class.get_config()).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn mapped(family: Family, non_default: &str, optional: &str, model: &str) -> Object {
    map_openai_params(MappingRequest {
        family,
        non_default: &object(non_default),
        optional: object(optional),
        model,
        drop_params: false,
        global_drop_params: false,
    })
}

fn titan_stop(stop: &str, method_drop_params: bool, global_drop_params: bool) -> Object {
    map_openai_params(MappingRequest {
        family: Family::Titan,
        non_default: &object(&format!(r#"{{"stop": {stop}}}"#)),
        optional: object(r#"{"stopSequences": "prior"}"#),
        model: "amazon.titan-text-express-v1",
        drop_params: method_drop_params,
        global_drop_params,
    })
}

fn ai21_values(text: &str) -> Ai21Values {
    serde_json::from_str(text).unwrap()
}

fn cohere_values(text: &str) -> CohereValues {
    serde_json::from_str(text).unwrap()
}

fn llama_values(text: &str) -> LlamaValues {
    serde_json::from_str(text).unwrap()
}

fn mistral_values(text: &str) -> MistralValues {
    serde_json::from_str(text).unwrap()
}

fn titan_values(text: &str) -> TitanValues {
    serde_json::from_str(text).unwrap()
}

fn anthropic_values(text: &str) -> AnthropicValues {
    serde_json::from_str(text).unwrap()
}

fn assert_supported(family: Family, expected: &[&str]) {
    assert_eq!(supported_openai_params(family), expected);
}

fn assert_malformed(response: &str, field: &str) {
    match output_text(&object(response)) {
        Err(MistralOutputError::Malformed { field: reported }) => assert_eq!(reported, field),
        Ok(output) => panic!("expected a malformed {field} error, got {:?}", output.text),
        Err(other) => panic!("expected a malformed {field} error, got {other}"),
    }
}

fn ai21_penalties() {
    assert_supported(
        Family::Ai21,
        &["max_tokens", "temperature", "top_p", "stream"],
    );
    let mapped = mapped(
        Family::Ai21,
        r#"{"max_tokens": 5, "temperature": false, "top_p": 0.25, "stream": null,
            "max_completion_tokens": 9, "stop": ["x"], "frequencePenalty": {"scale": 0}}"#,
        r#"{"temperature": 0.1, "keep": true}"#,
        "ai21.j2-mid-v1",
    );
    assert_eq!(
        mapped,
        object(
            r#"{"temperature": false, "keep": true, "maxTokens": 5, "topP": 0.25, "stream": null}"#
        )
    );

    let mut class = LegacyConfig::new(Family::Ai21, 4096);
    apply(
        &mut class.attributes,
        &ai21_values(
            r#"{"maxTokens": 0, "frequencePenalty": {"scale": 0}, "presencePenalty": {"scale": 1},
                "countPenalty": {"scale": 2}}"#,
        ),
    )
    .unwrap();
    assert_eq!(
        projected(&class),
        object(
            r#"{"maxTokens": 0, "frequencePenalty": {"scale": 0}, "presencePenalty": {"scale": 1},
                "countPenalty": {"scale": 2}}"#
        )
    );
}

fn cohere_max_tokens() {
    assert_supported(
        Family::Cohere,
        &[
            "stream",
            "temperature",
            "max_tokens",
            "max_completion_tokens",
            "top_p",
            "frequency_penalty",
            "presence_penalty",
            "stop",
            "n",
            "tools",
            "tool_choice",
            "seed",
            "extra_headers",
        ],
    );
    let token_after = mapped(
        Family::Cohere,
        r#"{"max_tokens": 5, "max_completion_tokens": 9}"#,
        "{}",
        "cohere.command-text-v14",
    );
    assert_eq!(token_after, object(r#"{"max_tokens": 9}"#));
    let token_before = mapped(
        Family::Cohere,
        r#"{"max_completion_tokens": 9, "max_tokens": 5}"#,
        "{}",
        "cohere.command-text-v14",
    );
    assert_eq!(token_before, object(r#"{"max_tokens": 5}"#));
    cohere_parameter_mapping();
}

fn cohere_parameter_mapping() {
    let mapped = mapped(
        Family::Cohere,
        r#"{"stream": true, "temperature": 0.2, "frequency_penalty": 0.3, "presence_penalty": 0.4,
            "tools": [], "seed": 7, "n": 2, "top_p": 0.5, "stop": "END", "tool_choice": "auto",
            "extra_headers": {"x": "y"}, "unknown": 1}"#,
        r#"{"p": 0.9}"#,
        "cohere.command-text-v14",
    );
    assert_eq!(
        mapped,
        object(
            r#"{"stream": true, "temperature": 0.2, "frequency_penalty": 0.3, "presence_penalty": 0.4,
                "tools": [], "seed": 7, "p": 0.5, "num_generations": 2, "stop_sequences": "END"}"#
        )
    );
}

fn llama_generation_length() {
    assert_supported(
        Family::Llama,
        &["max_tokens", "temperature", "top_p", "stream"],
    );
    let mapped = mapped(
        Family::Llama,
        r#"{"max_tokens": 64, "temperature": 0.5, "top_p": 0.9, "stream": false,
            "max_completion_tokens": 12, "stop": ["x"]}"#,
        "{}",
        "meta.llama3-8b-instruct-v1:0",
    );
    assert_eq!(
        mapped,
        object(r#"{"max_gen_len": 64, "temperature": 0.5, "top_p": 0.9, "stream": false}"#)
    );

    let mut class = LegacyConfig::new(Family::Llama, 4096);
    apply(
        &mut class.attributes,
        &llama_values(r#"{"maxTokenCount": 32, "topP": 1}"#),
    )
    .unwrap();
    assert_eq!(
        projected(&class),
        object(r#"{"maxTokenCount": 32, "topP": 1}"#)
    );
    assert!(!mapped.contains_key("maxTokenCount"));
    assert!(!mapped.contains_key("topP"));
}

fn mistral_max_tokens() {
    assert_supported(
        Family::Mistral,
        &["max_tokens", "temperature", "top_p", "stop", "stream"],
    );
    let mapped = mapped(
        Family::Mistral,
        r#"{"max_tokens": 16, "temperature": 0.1, "top_p": 0.2, "stop": ["END"], "stream": true,
            "top_k": 40, "unknown": 1}"#,
        r#"{"keep": 1}"#,
        "mistral.mistral-7b-instruct-v0:2",
    );
    assert_eq!(
        mapped,
        object(
            r#"{"keep": 1, "max_tokens": 16, "temperature": 0.1, "top_p": 0.2, "stop": ["END"],
                "stream": true}"#
        )
    );

    let mut class = LegacyConfig::new(Family::Mistral, 4096);
    apply(&mut class.attributes, &mistral_values(r#"{"top_k": 40}"#)).unwrap();
    assert_eq!(projected(&class), object(r#"{"top_k": 40}"#));
    assert!(!mapped.contains_key("top_k"));
}

fn mistral_output_text() {
    let choices = output_text(&object(
        r#"{"choices": [{"message": {"content": "from choices"}, "finish_reason": "tool_calls"}],
            "outputs": [{"text": "from outputs", "stop_reason": "end_turn"}]}"#,
    ))
    .unwrap();
    assert_eq!(choices.text, Json::Text("from choices".to_owned()));
    assert_eq!(choices.finish_reason, Json::Text("tool_calls".to_owned()));

    let outputs = output_text(&object(
        r#"{"outputs": [{"text": "from outputs", "stop_reason": "end_turn"}]}"#,
    ))
    .unwrap();
    assert_eq!(outputs.text, Json::Text("from outputs".to_owned()));
    assert_eq!(outputs.finish_reason, Json::Text("end_turn".to_owned()));

    let null_content = output_text(&object(
        r#"{"choices": [{"message": {"content": null}, "finish_reason": null}], "outputs": []}"#,
    ))
    .unwrap();
    assert_eq!(null_content.text, Json::Null);
    assert_eq!(null_content.finish_reason, Json::Null);

    mistral_output_unexpected();
    mistral_output_malformed();
}

fn mistral_output_unexpected() {
    match output_text(&object("{}")) {
        Err(MistralOutputError::Unexpected(error)) => {
            assert_eq!(error.error.status_code, 400);
            assert_eq!(
                error.error.message,
                "Unexpected mistral completion response"
            );
        }
        Ok(output) => panic!(
            "expected the unexpected-response error, got {:?}",
            output.text
        ),
        Err(other) => panic!("expected the unexpected-response error, got {other}"),
    }
}

fn mistral_output_malformed() {
    assert_malformed(r#"{"choices": []}"#, "choices");
    assert_malformed(r#"{"choices": {}}"#, "choices");
    assert_malformed(r#"{"choices": ["text"]}"#, "choices");
    assert_malformed(
        r#"{"choices": [{"finish_reason": "stop"}]}"#,
        "choices.message",
    );
    assert_malformed(
        r#"{"choices": [{"message": {"content": "x"}}]}"#,
        "choices.finish_reason",
    );
    assert_malformed(
        r#"{"outputs": [{"stop_reason": "end_turn"}]}"#,
        "outputs.text",
    );
    assert_malformed(r#"{"outputs": [{"text": "x"}]}"#, "outputs.stop_reason");
}

fn legacy_config_defaults() {
    for family in [
        Family::Ai21,
        Family::Cohere,
        Family::Llama,
        Family::Mistral,
        Family::Titan,
    ] {
        assert_eq!(projected(&LegacyConfig::new(family, 4096)), object("{}"));
    }
    assert_eq!(
        projected(&LegacyConfig::new(Family::Anthropic, 4096)),
        object(r#"{"max_tokens_to_sample": 4096, "_abc_impl": {"kind": "abc_data"}}"#)
    );
    assert_eq!(
        projected(&LegacyConfig::new(Family::Anthropic, 1024)),
        object(r#"{"max_tokens_to_sample": 1024, "_abc_impl": {"kind": "abc_data"}}"#)
    );
    legacy_config_overrides();
}

fn legacy_config_overrides() {
    let mut titan = LegacyConfig::new(Family::Titan, 4096);
    apply(
        &mut titan.attributes,
        &titan_values(r#"{"maxTokenCount": 0, "stopSequences": [], "temperature": 0, "topP": 0}"#),
    )
    .unwrap();
    apply(&mut titan.attributes, &titan_values("{}")).unwrap();
    assert_eq!(
        projected(&titan),
        object(r#"{"maxTokenCount": 0, "stopSequences": [], "temperature": 0, "topP": 0}"#)
    );

    let mut anthropic = LegacyConfig::new(Family::Anthropic, 4096);
    apply(
        &mut anthropic.attributes,
        &anthropic_values(r#"{"temperature": 0.5}"#),
    )
    .unwrap();
    assert_eq!(
        projected(&anthropic),
        object(
            r#"{"max_tokens_to_sample": 4096, "temperature": 0.5,
                "_abc_impl": {"kind": "abc_data"}}"#
        )
    );
    assert!(!anthropic.attributes.contains_key("anthropic_version"));
}

fn legacy_config_projection() {
    let mut ai21 = LegacyConfig::new(Family::Ai21, 4096);
    for (name, kind) in [
        ("function", AttributeKind::Function),
        ("builtin", AttributeKind::Builtin),
        ("class_method", AttributeKind::ClassMethod),
        ("static_method", AttributeKind::StaticMethod),
        ("descriptor", AttributeKind::Descriptor),
    ] {
        ai21.attributes
            .insert(name.to_owned(), ClassAttribute::Special { kind });
    }
    for (name, value) in [
        ("__dunder__", ConfigJson::Text("dir".to_owned())),
        ("nothing", ConfigJson::Null),
        ("_private", ConfigJson::Text("kept".to_owned())),
        ("_abc_value", ConfigJson::Text("abc".to_owned())),
    ] {
        ai21.attributes
            .insert(name.to_owned(), ClassAttribute::Value(value));
    }
    assert_eq!(
        projected(&ai21),
        object(r#"{"descriptor": {"kind": "descriptor"}, "_private": "kept"}"#)
    );

    projection_keeps_abc_values();
    projection_merges_family_values();
}

fn projection_keeps_abc_values() {
    let mut anthropic = LegacyConfig::new(Family::Anthropic, 4096);
    anthropic.attributes.insert(
        "_abc_value".to_owned(),
        ClassAttribute::Value(ConfigJson::Text("abc".to_owned())),
    );
    assert_eq!(
        projected(&anthropic),
        object(
            r#"{"max_tokens_to_sample": 4096, "_abc_value": "abc",
                "_abc_impl": {"kind": "abc_data"}}"#
        )
    );
}

fn projection_merges_family_values() {
    let mut cohere = LegacyConfig::new(Family::Cohere, 4096);
    apply(
        &mut cohere.attributes,
        &cohere_values(r#"{"max_tokens": 8, "return_likelihood": "ALL"}"#),
    )
    .unwrap();
    apply(
        &mut cohere.attributes,
        &cohere_values(r#"{"temperature": 0.1}"#),
    )
    .unwrap();
    assert_eq!(
        projected(&cohere),
        object(r#"{"max_tokens": 8, "return_likelihood": "ALL", "temperature": 0.1}"#)
    );
}

fn titan_stop_filter() {
    assert_supported(
        Family::Titan,
        &[
            "max_tokens",
            "max_completion_tokens",
            "stop",
            "temperature",
            "top_p",
            "stream",
        ],
    );
    let aliases = mapped(
        Family::Titan,
        r#"{"max_tokens": 5, "max_completion_tokens": 9, "top_p": 0.5, "stream": true,
            "temperature": 0.2}"#,
        "{}",
        "amazon.titan-text-express-v1",
    );
    assert_eq!(
        aliases,
        object(r#"{"maxTokenCount": 9, "topP": 0.5, "stream": true, "temperature": 0.2}"#)
    );
    let token_order = mapped(
        Family::Titan,
        r#"{"max_completion_tokens": 9, "max_tokens": 5}"#,
        "{}",
        "amazon.titan-text-express-v1",
    );
    assert_eq!(token_order, object(r#"{"maxTokenCount": 5}"#));

    titan_filtered_stop_sequences();
    titan_stop_argument_kept();
}

fn titan_filtered_stop_sequences() {
    let kept = titan_stop(
        r#"["|", "x", "User:", "|\n", "User:\n", "||", "", "user:", "User:\n\n", 5]"#,
        false,
        true,
    );
    assert_eq!(
        kept,
        object(r#"{"stopSequences": ["|", "User:", "|\n", "User:\n", "||"]}"#)
    );
    assert_eq!(
        titan_stop(r#""END""#, false, true),
        object(r#"{"stopSequences": []}"#)
    );
    assert_eq!(
        titan_stop("null", false, true),
        object(r#"{"stopSequences": []}"#)
    );
    assert_eq!(
        titan_stop(r#""END""#, true, false),
        object(r#"{"stopSequences": "END"}"#)
    );
    assert_eq!(
        titan_stop(r#"["|"]"#, true, false),
        object(r#"{"stopSequences": ["|"]}"#)
    );
}

fn stop_argument_after(provider: &str, model: &str, supported: &str) -> Object {
    let mut arguments = object(supported);
    map_and_modify_arg(
        &mut arguments,
        true,
        provider,
        model,
        &Json::Text("END".to_owned()),
    );
    arguments
}

fn titan_stop_argument_kept() {
    assert_eq!(
        stop_argument_after("bedrock", "cohere.command-text-v14", r#"{"stop": "END"}"#),
        object(r#"{"stop": "END"}"#)
    );
    assert_eq!(
        stop_argument_after(
            "bedrock",
            "Amazon.titan-text-express-v1",
            r#"{"stop": "END"}"#
        ),
        object(r#"{"stop": "END"}"#)
    );
    assert_eq!(
        stop_argument_after(
            "vertex",
            "amazon.titan-text-express-v1",
            r#"{"stop": "END"}"#
        ),
        object(r#"{"stop": "END"}"#)
    );
    assert_eq!(
        stop_argument_after("bedrock", "amazon.titan-text-express-v1", "{}"),
        object("{}")
    );
}

fn claude2_parameters() {
    assert_supported(
        Family::Anthropic,
        &[
            "max_tokens",
            "max_completion_tokens",
            "temperature",
            "stop",
            "top_p",
            "stream",
        ],
    );
    let mapped = mapped(
        Family::Anthropic,
        r#"{"max_tokens": 5, "max_completion_tokens": 9, "temperature": 0.3, "stop": ["\n\nHuman:"],
            "top_p": 0.8, "unknown": 1}"#,
        "{}",
        "anthropic.claude-v2",
    );
    assert_eq!(
        mapped,
        object(
            r#"{"max_tokens_to_sample": 9, "temperature": 0.3, "stop_sequences": ["\n\nHuman:"],
                "top_p": 0.8}"#
        )
    );
    let token_order = mapped(
        Family::Anthropic,
        r#"{"max_completion_tokens": 9, "max_tokens": 5}"#,
        "{}",
        "anthropic.claude-v2",
    );
    assert_eq!(token_order, object(r#"{"max_tokens_to_sample": 5}"#));

    claude2_stream_rule();
}

fn claude2_stream_rule() {
    assert_eq!(
        mapped(
            Family::Anthropic,
            r#"{"stream": true}"#,
            r#"{"stream": false}"#,
            "anthropic.claude-v2"
        ),
        object(r#"{"stream": true}"#)
    );
    assert_eq!(
        mapped(
            Family::Anthropic,
            r#"{"stream": false}"#,
            r#"{"stream": true}"#,
            "anthropic.claude-v2"
        ),
        object(r#"{"stream": true}"#)
    );
    assert_eq!(
        mapped(
            Family::Anthropic,
            r#"{"stream": null}"#,
            r#"{"stream": true}"#,
            "anthropic.claude-v2"
        ),
        object(r#"{"stream": true}"#)
    );
    assert_eq!(
        mapped(
            Family::Anthropic,
            r#"{"stream": 1}"#,
            "{}",
            "anthropic.claude-v2"
        ),
        object("{}")
    );
}

#[rstest]
#[case::ai21_penalties(ai21_penalties)]
#[case::cohere_max_tokens(cohere_max_tokens)]
#[case::llama_generation_length(llama_generation_length)]
#[case::mistral_max_tokens(mistral_max_tokens)]
#[case::mistral_output_text(mistral_output_text)]
#[case::legacy_config_defaults(legacy_config_defaults)]
#[case::legacy_config_projection(legacy_config_projection)]
#[case::titan_stop_filter(titan_stop_filter)]
#[case::claude2_parameters(claude2_parameters)]
fn legacy_models_cases(#[case] run: fn()) {
    run();
}
