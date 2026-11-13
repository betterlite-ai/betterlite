use betterlite_models::prompt_contracts::{PromptAsyncCompileRequest, PromptCompileRequest};
use rstest::rstest;

use crate::anthropic_cache_callbacks::INTEGRATION_NAME;
use crate::anthropic_cache_callbacks::result::compile_prompt_helper;
use crate::orchestration::prompt_orchestration::PromptManagementBackend as _;

use super::support::{dynamic_params, empty_dynamic_params, plain_hook, prompt_spec};

#[test]
fn the_integration_name_is_the_cache_control_callback_name() {
    assert_eq!(INTEGRATION_NAME, "anthropic_cache_control_hook");
    assert_eq!(
        plain_hook().integration_name(),
        "anthropic_cache_control_hook"
    );
}

#[rstest]
#[case::without_a_prompt(None, false, false)]
#[case::with_a_prompt(Some("greeting"), false, false)]
#[case::with_an_empty_prompt_id(Some(""), false, false)]
#[case::with_a_prompt_spec(Some("greeting"), true, false)]
#[case::with_call_dynamic_params(None, false, true)]
#[case::with_a_spec_and_dynamic_params(Some("greeting"), true, true)]
fn should_run_prompt_management_is_false_for_every_request(
    #[case] prompt_id: Option<&str>,
    #[case] with_spec: bool,
    #[case] with_dynamic_params: bool,
) {
    let spec = prompt_spec();
    let dynamic = if with_dynamic_params {
        dynamic_params()
    } else {
        empty_dynamic_params()
    };
    let hook = plain_hook();
    let prompt_spec = with_spec.then_some(&spec);
    assert!(!hook.should_run(prompt_id, prompt_spec, &dynamic));
}

#[rstest]
#[case::without_a_prompt(None)]
#[case::with_a_prompt(Some("greeting"))]
#[case::with_an_empty_prompt_id(Some(""))]
fn compile_prompt_helper_returns_an_empty_client(#[case] prompt_id: Option<&str>) {
    let dynamic = dynamic_params();
    let spec = prompt_spec();
    let client = compile_prompt_helper(&PromptCompileRequest {
        prompt_id,
        prompt_spec: Some(&spec),
        prompt_variables: None,
        dynamic_callback_params: &dynamic,
        prompt_label: Some("production"),
        prompt_version: Some(2),
    });
    assert_eq!(client.prompt_id.as_deref(), prompt_id);
    assert!(client.prompt_template.is_empty());
    assert_eq!(client.prompt_template_model, None);
    assert_eq!(client.prompt_optional_params, None);
    assert_eq!(client.completed_messages, None);
}

#[rstest]
#[case::without_a_prompt(None)]
#[case::with_a_prompt(Some("greeting"))]
#[case::with_an_empty_prompt_id(Some(""))]
#[tokio::test]
async fn async_compile_prompt_helper_matches_the_sync_helper(#[case] prompt_id: Option<&str>) {
    let dynamic = dynamic_params();
    let spec = prompt_spec();
    let hook = plain_hook();
    let sync_request = PromptCompileRequest {
        prompt_id,
        prompt_spec: Some(&spec),
        prompt_variables: None,
        dynamic_callback_params: &dynamic,
        prompt_label: Some("production"),
        prompt_version: Some(2),
    };
    let async_request = PromptAsyncCompileRequest {
        prompt_id,
        prompt_variables: None,
        dynamic_callback_params: &dynamic,
        prompt_spec: Some(&spec),
        prompt_label: Some("production"),
        prompt_version: Some(2),
    };
    let sync = compile_prompt_helper(&sync_request);
    let asynchronous = hook
        .compile_template(&async_request)
        .await
        .expect("the hook never fails");
    assert_eq!(asynchronous, sync);
}
