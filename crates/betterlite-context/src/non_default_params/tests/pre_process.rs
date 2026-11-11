#![cfg(test)]

use betterlite_models::controls::json_value::JsonValue;
use rstest::rstest;

use super::params;
use crate::non_default_params::Params;
use crate::non_default_params::error::{MapperError, NonDefaultParamsError};
use crate::non_default_params::mapper::OptionalParamsMapper;
use crate::non_default_params::names::{DEFAULT_CHAT_COMPLETION_PARAMS, OPENAI_EMBEDDING_PARAMS};
use crate::non_default_params::pre_process::{
    PreProcessOptions, PreProcessRequest, base_pre_process_params, pre_process_non_default_params,
    remove_sensitive_keys,
};
use crate::non_default_params::schema_cleanup::point_refs_at_root_defs;

const PLAIN: PreProcessOptions = PreProcessOptions {
    remove_sensitive_keys: false,
    add_provider_specific_params: false,
};

fn request<'request>(
    passed: &'request Params,
    special: &'request Params,
    provider: &'request str,
    drops: &'request [String],
) -> PreProcessRequest<'request> {
    PreProcessRequest {
        passed_params: passed,
        special_params: special,
        custom_llm_provider: provider,
        additional_drop_params: drops,
        provider_config: None,
    }
}

struct RootDefsConfig;

impl OptionalParamsMapper for RootDefsConfig {
    fn supported_openai_params(&self, _model: &str) -> Vec<String> {
        Vec::new()
    }

    fn map_openai_params(
        &self,
        _non_default_params: &Params,
        optional_params: Params,
        _model: &str,
        _drop_params: bool,
    ) -> Result<Params, MapperError> {
        Ok(optional_params)
    }

    fn map_response_format(&self, response_format: &mut JsonValue) {
        point_refs_at_root_defs(response_format);
    }
}

#[rstest]
#[case::bedrock_keeps_aws("bedrock", "aws_region_name", true)]
#[case::sagemaker_chat_keeps_aws("sagemaker_chat", "aws_region_name", true)]
#[case::sagemaker_keeps_aws("sagemaker", "aws_access_key_id", true)]
#[case::openai_drops_aws("openai", "aws_region_name", false)]
#[case::bedrock_like_name_drops_aws("bedrock_converse", "aws_region_name", false)]
#[case::project_id_is_never_a_body_field("bedrock", "aws_bedrock_project_id", false)]
#[case::sagemaker_keeps_hf_model_name("sagemaker", "hf_model_name", true)]
#[case::sagemaker_chat_drops_hf_model_name("sagemaker_chat", "hf_model_name", false)]
#[case::bedrock_drops_hf_model_name("bedrock", "hf_model_name", false)]
#[case::vertex_ai_keeps_vertex("vertex_ai", "vertex_project", true)]
#[case::vertex_ai_beta_keeps_vertex("vertex_ai_beta", "vertex_location", true)]
#[case::gdc_keeps_vertex("gdc", "vertex_project", true)]
#[case::openai_drops_vertex("openai", "vertex_project", false)]
#[case::other_keys_pass("openai", "some_other_key", true)]
fn pre_process_special_param_filter(#[case] provider: &str, #[case] key: &str, #[case] kept: bool) {
    let special = params(&format!(r#"{{"{key}": "v"}}"#));
    let passed = params("{}");
    let processed = base_pre_process_params(
        &request(&passed, &special, provider, &[]),
        DEFAULT_CHAT_COMPLETION_PARAMS,
        &["messages"],
    );
    assert_eq!(processed.passed_params.contains_key(key), kept);
}

#[test]
fn pre_process_filter_selects_by_the_chat_defaults() {
    let passed = params(
        r#"{"model": "m", "custom_llm_provider": "p", "api_version": "v", "drop_params": true,
            "allowed_openai_params": ["x"], "additional_drop_params": ["user"],
            "messages": [{"role": "user"}], "temperature": 0.1, "user": "u", "top_k": 1,
            "n": null, "stream": false}"#,
    );
    let special = params(r#"{"aws_region_name": "r"}"#);
    let drops = vec!["user".to_owned()];
    let processed = base_pre_process_params(
        &request(&passed, &special, "openai", &drops),
        DEFAULT_CHAT_COMPLETION_PARAMS,
        &["messages"],
    );
    assert_eq!(
        processed.non_default_params,
        params(r#"{"temperature": 0.1, "stream": false}"#)
    );
    assert!(!processed.passed_params.contains_key("aws_region_name"));
    assert!(processed.passed_params.contains_key("top_k"));
}

#[test]
fn pre_process_filter_selects_by_the_embedding_defaults() {
    let passed = params(
        r#"{"dimensions": 3, "encoding_format": null, "user": "u", "temperature": 1, "input": "x"}"#,
    );
    let special = params("{}");
    let processed = base_pre_process_params(
        &request(&passed, &special, "openai", &[]),
        OPENAI_EMBEDDING_PARAMS,
        &["input"],
    );
    assert_eq!(
        processed.non_default_params,
        params(r#"{"dimensions": 3, "user": "u"}"#)
    );
}

#[test]
fn pre_process_filter_keeps_the_function_schema_untouched() {
    let passed = params(
        r#"{"tools": [{"function": {"parameters": {"additionalProperties": false}}}], "temperature": 1}"#,
    );
    let special = params("{}");
    let result =
        pre_process_non_default_params(&request(&passed, &special, "gemini", &[]), PLAIN).unwrap();
    assert_eq!(result, passed);
}

#[rstest]
#[case::openai_uses_extra_body("openai", r#"{"temperature": 1, "extra_body": {"top_k": 3}}"#)]
#[case::anthropic_uses_top_level("anthropic", r#"{"temperature": 1, "top_k": 3}"#)]
fn pre_process_filter_adds_provider_specific_params(
    #[case] provider: &str,
    #[case] expected: &str,
) {
    let passed = params(r#"{"temperature": 1, "top_k": 3}"#);
    let special = params("{}");
    let options = PreProcessOptions {
        remove_sensitive_keys: false,
        add_provider_specific_params: true,
    };
    let result =
        pre_process_non_default_params(&request(&passed, &special, provider, &[]), options)
            .unwrap();
    assert_eq!(result, params(expected));
}

#[rstest]
#[case::removed_when_asked(true, r#"{"temperature": 1}"#)]
#[case::kept_when_not_asked(false, r#"{"temperature": 1, "my_secret": "s"}"#)]
fn pre_process_sensitive_keys_follow_the_option(#[case] remove: bool, #[case] expected: &str) {
    let passed = params(r#"{"temperature": 1, "my_secret": "s"}"#);
    let special = params("{}");
    let options = PreProcessOptions {
        remove_sensitive_keys: remove,
        add_provider_specific_params: true,
    };
    let result =
        pre_process_non_default_params(&request(&passed, &special, "anthropic", &[]), options)
            .unwrap();
    assert_eq!(result, params(expected));
}

#[test]
fn pre_process_filter_rejects_a_response_format_that_is_not_an_object() {
    let passed = params(r#"{"response_format": "json"}"#);
    let special = params("{}");
    let error = pre_process_non_default_params(&request(&passed, &special, "openai", &[]), PLAIN)
        .unwrap_err();
    assert!(matches!(
        error,
        NonDefaultParamsError::WrongType("response_format", _)
    ));
    assert_eq!(error.status_code(), 400);
}

#[rstest]
#[case::key_phrase(r#"{"api_key": 1, "n": 1}"#, r#"{"n": 1}"#)]
#[case::secret_phrase(r#"{"client_secret": 1, "n": 1}"#, r#"{"n": 1}"#)]
#[case::access_phrase(r#"{"access_token": 1, "n": 1}"#, r#"{"n": 1}"#)]
#[case::credential_phrase(r#"{"vertex_credentials": 1, "n": 1}"#, r#"{"n": 1}"#)]
#[case::upper_case_is_matched(r#"{"API_KEY": 1, "Secret": 2, "n": 1}"#, r#"{"n": 1}"#)]
#[case::substring_is_matched(r#"{"monkey": 1, "n": 1}"#, r#"{"n": 1}"#)]
#[case::other_names_stay(r#"{"temperature": 1, "n": 1}"#, r#"{"temperature": 1, "n": 1}"#)]
fn sensitive_keys_are_removed_by_phrase(#[case] input: &str, #[case] expected: &str) {
    let mut values = params(input);
    remove_sensitive_keys(&mut values);
    assert_eq!(values, params(expected));
}

const REF_FORMAT: &str = r##"{"response_format": {"type": "json_schema", "json_schema": {"schema": {"$ref": "#/$defs/Item"}}}}"##;

#[rstest]
#[case::config_rewrites_the_refs(true, r#"{"response_format": {"type": "json_schema", "json_schema": {"schema": {"$ref": "/$defs/Item"}}}}"#)]
#[case::no_config_keeps_the_refs(false, REF_FORMAT)]
fn pre_process_response_format_goes_through_the_provider_config(
    #[case] with_config: bool,
    #[case] expected: &str,
) {
    let passed = params(REF_FORMAT);
    let special = params("{}");
    let mut processed = request(&passed, &special, "anthropic", &[]);
    if with_config {
        processed.provider_config = Some(&RootDefsConfig);
    }
    let result = pre_process_non_default_params(&processed, PLAIN).unwrap();
    assert_eq!(result, params(expected));
}

#[test]
fn pre_process_provider_config_adds_no_response_format() {
    let passed = params(r#"{"temperature": 1}"#);
    let special = params("{}");
    let mut processed = request(&passed, &special, "anthropic", &[]);
    processed.provider_config = Some(&RootDefsConfig);
    let result = pre_process_non_default_params(&processed, PLAIN).unwrap();
    assert_eq!(result, passed);
}
