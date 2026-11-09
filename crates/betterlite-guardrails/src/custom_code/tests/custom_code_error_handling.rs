use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::fake::ALLOW_SOURCE;
use super::fake::FakeHost;
use super::fake::LoadReply;
use super::fake::build;
use super::fake::exiting;
use super::fake::failing;
use super::fake::inputs;
use super::fake::params_with_timeout;
use super::fake::spinning;
use super::fake::text;
use crate::custom_code::error::CustomCodeApplyError;
use crate::custom_code::error::CustomCodeGuardrailError;
use crate::generic_guardrail_contracts::GuardrailInputType;

async fn execution_error(host: &Arc<FakeHost>) -> CustomCodeGuardrailError {
    let guardrail = build(params_with_timeout(ALLOW_SOURCE, 0.03), host);
    let mut request_data = BTreeMap::new();
    let error = guardrail
        .apply_guardrail(
            inputs(&["hello"]),
            &mut request_data,
            GuardrailInputType::Request,
        )
        .await
        .unwrap_err();
    let CustomCodeApplyError::Execution(failure) = error else {
        panic!("expected an execution error");
    };
    failure
}

#[tokio::test]
async fn a_timeout_names_the_guardrail_and_reports_the_budget_in_the_details() {
    let host = FakeHost::new(LoadReply::Module(spinning()));
    let error = execution_error(&host).await;
    assert!(matches!(error, CustomCodeGuardrailError::Timeout { .. }));
    assert_eq!(
        error.to_string(),
        "Custom code guardrail 'custom_code' exceeded its 0.03s execution timeout"
    );
    let details = error.details().unwrap();
    assert_eq!(details.get("guardrail_name"), Some(&text("custom_code")));
    assert_eq!(details.get("input_type"), Some(&text("request")));
    assert_eq!(details.get("timeout"), Some(&GuardrailValue::Decimal(0.03)));
}

#[tokio::test]
async fn a_failure_inside_the_code_is_an_execution_error_with_the_cause() {
    let host = FakeHost::new(LoadReply::Module(failing("boom")));
    let error = execution_error(&host).await;
    assert!(matches!(error, CustomCodeGuardrailError::Execution { .. }));
    assert_eq!(
        error.to_string(),
        "Custom code guardrail execution failed: boom"
    );
    let details = error.details().unwrap();
    assert_eq!(details.get("guardrail_name"), Some(&text("custom_code")));
    assert_eq!(details.get("input_type"), Some(&text("request")));
    assert!(!details.contains_key("timeout"));
}

#[tokio::test]
async fn a_sandbox_exit_is_an_execution_error_with_the_exit_text() {
    let host = FakeHost::new(LoadReply::Module(exiting()));
    let error = execution_error(&host).await;
    assert!(error.to_string().contains("SystemExit: bye"));
}
