#![cfg(test)]

use super::{
    _get_model_from_prompt as model_helper, CustomPromptManagement, PromptIgnoreFlags,
    PromptManagement, PromptManagementClient, PromptParams, PromptProcessingOptions,
    post_compile_prompt_processing, resolve_ignore_flags,
};
use crate::decisions::Json;
use crate::prompt_spec::PromptSpec;

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("valid JSON")
}

fn spec(params: &str) -> PromptSpec {
    serde_json::from_str(&format!(
        r#"{{"prompt_id":"p","litellm_params":{{"prompt_integration":"custom"{params}}}}}"#
    ))
    .expect("valid prompt spec")
}

#[test]
fn ignore_flags_use_or_semantics() {
    let prompt_spec = spec(r#", "ignore_prompt_manager_model": true"#);
    assert_eq!(
        resolve_ignore_flags(
            Some(&prompt_spec),
            PromptIgnoreFlags {
                model: false,
                optional_params: true,
            },
        ),
        PromptIgnoreFlags {
            model: true,
            optional_params: true,
        }
    );
    assert_eq!(
        resolve_ignore_flags(None, PromptIgnoreFlags::default()),
        PromptIgnoreFlags::default()
    );
}

#[test]
fn model_prefix_is_stripped_repeatedly_without_template_override() {
    let client = PromptManagementClient::default();
    assert_eq!(
        model_helper(&client, "custom", "custom/custom/model"),
        "model"
    );
}

#[test]
fn processing_uses_template_params_and_completed_messages() {
    let mut request_params = PromptParams::new();
    request_params.insert("temperature".to_owned(), json("0.2"));
    let mut template_params = PromptParams::new();
    template_params.insert("temperature".to_owned(), json("0.8"));
    template_params.insert("max_tokens".to_owned(), json("10"));
    let client = PromptManagementClient {
        prompt_id: Some("p".to_owned()),
        prompt_template: vec![json(r#"{"role":"system"}"#)],
        prompt_template_model: None,
        prompt_optional_params: Some(template_params),
        completed_messages: Some(vec![json(r#"{"role":"user"}"#)]),
    };
    let (model, messages, params) = post_compile_prompt_processing(
        &client,
        vec![json(r#"{"role":"client"}"#)],
        request_params,
        "custom/model".to_owned(),
        &PromptProcessingOptions {
            integration_name: "custom".to_owned(),
            ignore_flags: PromptIgnoreFlags::default(),
        },
    );
    assert_eq!(model, "model");
    assert_eq!(messages, vec![json(r#"{"role":"user"}"#)]);
    assert_eq!(params.get("temperature"), Some(&json("0.8")));
    assert_eq!(params.get("max_tokens"), Some(&json("10")));
}

#[test]
fn custom_management_is_noop_gate_and_rejects_compilation() {
    let manager = CustomPromptManagement;
    let dynamic_params = json("{}");
    assert_eq!(manager.integration_name(), "custom-prompt-management");
    assert!(manager.should_run_prompt_management(None, None, &dynamic_params));
    let request = super::PromptCompileRequest {
        prompt_id: None,
        prompt_spec: None,
        prompt_variables: None,
        dynamic_callback_params: &dynamic_params,
        prompt_label: None,
        prompt_version: None,
    };
    let compile_helper: fn(
        &CustomPromptManagement,
        &super::PromptCompileRequest<'_>,
    ) -> Result<PromptManagementClient, super::PromptContractError> =
        <CustomPromptManagement as PromptManagement>::_compile_prompt_helper;
    assert_eq!(
        compile_helper(&manager, &request),
        Err(super::PromptContractError::NotImplemented)
    );
}

#[test]
fn prompt_request_deserializes_optional_fields() {
    let request: super::TestPromptRequest = serde_json::from_str(
        r#"{"dotprompt_content":"hello","prompt_variables":{"name":"Ada"},"conversation_history":[{"role":"user","content":"hi"}]}"#,
    )
    .expect("valid test prompt request");
    assert_eq!(request.dotprompt_content, "hello");
    assert!(request.prompt_variables.is_some());
    assert_eq!(request.conversation_history.as_ref().map(Vec::len), Some(1));
}
