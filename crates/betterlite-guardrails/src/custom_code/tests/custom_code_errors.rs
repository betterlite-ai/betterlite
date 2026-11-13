use std::collections::BTreeMap;
use std::error::Error;
use std::time::Duration;

use super::fake::text;
use crate::custom_code::error::CustomCodeApplyError;
use crate::custom_code::error::CustomCodeGuardrailError;
use crate::custom_code::error::ErrorDetails;
use crate::custom_code_execution::execution::error::ExecutionError;
use crate::custom_code_execution::sandbox::error::CompileError;

#[test]
fn compilation_and_execution_errors_keep_their_cause_and_details() {
    let compilation = CustomCodeGuardrailError::Syntax {
        source: CompileError::syntax("<guardrail>", 1, "invalid syntax"),
    };
    assert!(
        compilation
            .to_string()
            .starts_with("Syntax error in custom code:")
    );
    assert!(Error::source(&compilation).is_some());
    assert!(compilation.details().is_none());

    let mut details = ErrorDetails::new();
    details.insert("guardrail_name".to_owned(), text("custom_code"));
    let execution = CustomCodeGuardrailError::Execution {
        source: ExecutionError::TimeoutOutOfRange {
            timeout: Duration::from_secs(1),
        },
        details,
    };
    assert_eq!(
        execution.to_string(),
        "Custom code guardrail execution failed: the execution timeout 1s is too large to schedule"
    );
    assert!(Error::source(&execution).is_some());
    assert_eq!(execution.details().map(BTreeMap::len), Some(1));
}

#[test]
fn errors_raised_before_execution_carry_no_details() {
    assert_eq!(
        CustomCodeGuardrailError::MissingGuardrailName.to_string(),
        "Custom code guardrail requires a guardrail_name"
    );
    assert_eq!(
        CustomCodeGuardrailError::MissingCustomCode.to_string(),
        "Custom code guardrail requires 'custom_code' in litellm_params"
    );
    assert_eq!(
        CustomCodeGuardrailError::NonPositiveTimeout { timeout: -2.0_f64 }.to_string(),
        "execution_timeout must be positive, got -2"
    );
    assert!(
        CustomCodeGuardrailError::NonPositiveTimeout { timeout: 0.0_f64 }
            .details()
            .is_none()
    );
}

#[test]
fn a_blocked_response_is_a_400_with_litellms_detail_object() {
    let mut detection_info = ErrorDetails::new();
    detection_info.insert("matched_phrase".to_owned(), text("no"));
    let error = CustomCodeApplyError::Blocked {
        reason: "no".to_owned(),
        guardrail: "custom_code".to_owned(),
        detection_info,
    };
    assert_eq!(error.status_code(), Some(reqwest::StatusCode::BAD_REQUEST));
    let detail = error.block_detail().unwrap();
    assert_eq!(detail.error, "no");
    assert_eq!(detail.guardrail, "custom_code");
    assert_eq!(
        detail.detection_info.get("matched_phrase"),
        Some(&text("no"))
    );
}

#[test]
fn a_passthrough_and_an_execution_error_have_no_block_detail() {
    let passthrough = CustomCodeApplyError::Passthrough {
        message: "no".to_owned(),
        detection_info: ErrorDetails::new(),
    };
    assert!(passthrough.status_code().is_none());
    assert!(passthrough.block_detail().is_none());

    let execution = CustomCodeApplyError::Execution(CustomCodeGuardrailError::MissingCustomCode);
    assert_eq!(
        execution.status_code(),
        Some(reqwest::StatusCode::INTERNAL_SERVER_ERROR)
    );
    assert!(execution.block_detail().is_none());
}
