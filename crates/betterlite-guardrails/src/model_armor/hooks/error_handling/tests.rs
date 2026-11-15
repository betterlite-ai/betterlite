#![cfg(test)]

use indexmap::IndexMap;
use rstest::rstest;

use super::super::tests::support::{CLEAN, blocked, text, value};
use super::{FieldError, HookError, call_outcome, hook_outcome};
use crate::guardrail_selection::error::GuardrailSelectionError;
use crate::model_armor::decisions::ModelArmorAPIError;
use crate::model_armor::decisions::wire::SanitizeResponse;
use crate::model_armor::request_files::ScanError;
use crate::model_armor::transport::ArmorResponse;
use crate::model_armor::transport::error::{ArmorCallError, ArmorTokenError, TransportError};

fn api_error() -> ArmorCallError<ArmorTokenError> {
    ArmorCallError::Api(ModelArmorAPIError {
        detail: "Model Armor API error (upstream 400)".to_owned(),
    })
}

fn transport_error() -> ArmorCallError<ArmorTokenError> {
    ArmorCallError::Transport(TransportError::MissingTemplate)
}

fn response() -> ArmorResponse {
    ArmorResponse {
        body: value(CLEAN),
        verdict: SanitizeResponse::default(),
    }
}

fn block() -> HookError<ArmorTokenError> {
    HookError::Blocked {
        status: 400,
        detail: IndexMap::from([("error".to_owned(), text("Content blocked by Model Armor"))]),
    }
}

#[test]
fn error_handling_a_response_passes_through() {
    let outcome = call_outcome::<ArmorTokenError>(Ok(response()), true, "pre-call");
    assert!(outcome.expect("response").is_some());
}

#[rstest]
#[case::fail_closed(true)]
#[case::fail_open(false)]
fn error_handling_api_errors_follow_fail_on_error(#[case] fail_on_error: bool) {
    let outcome = call_outcome(Err(api_error()), fail_on_error, "pre-call");
    if fail_on_error {
        assert!(matches!(outcome, Err(HookError::Api(_))));
    } else {
        assert!(outcome.expect("fail open").is_none());
    }
}

#[rstest]
#[case::fail_closed(true)]
#[case::fail_open(false)]
fn error_handling_transport_errors_follow_fail_on_error(#[case] fail_on_error: bool) {
    let outcome = call_outcome(Err(transport_error()), fail_on_error, "pre-call");
    if fail_on_error {
        assert!(matches!(
            outcome,
            Err(HookError::Transport(TransportError::MissingTemplate))
        ));
    } else {
        assert!(outcome.expect("fail open").is_none());
    }
}

#[test]
fn error_handling_hook_outcome_passes_a_success() {
    hook_outcome::<ArmorTokenError>(Ok(()), true, "pre-call").expect("success");
    hook_outcome::<ArmorTokenError>(Ok(()), false, "pre-call").expect("success");
}

#[rstest]
#[case::fail_closed(true)]
#[case::fail_open(false)]
fn error_handling_a_block_is_never_swallowed(#[case] fail_on_error: bool) {
    let outcome = hook_outcome(Err(block()), fail_on_error, "pre-call");
    let (status, detail) = blocked(outcome.expect_err("blocked"));
    assert_eq!(status, 400);
    assert_eq!(
        detail.get("error"),
        Some(&text("Content blocked by Model Armor"))
    );
}

#[test]
fn error_handling_a_selection_error_is_never_swallowed() {
    let error = HookError::Selection(GuardrailSelectionError::TagMode {
        guardrail_name: Some("armor".to_owned()),
    });
    let outcome = hook_outcome(Err(error), false, "pre-call");
    assert!(matches!(outcome, Err(HookError::Selection(_))));
}

#[rstest]
#[case::fail_closed(true)]
#[case::fail_open(false)]
fn error_handling_other_errors_follow_fail_on_error(#[case] fail_on_error: bool) {
    let error = HookError::Field(FieldError {
        field: "messages",
        expected: "a list of messages with a string role",
    });
    let outcome = hook_outcome(Err(error), fail_on_error, "pre-call");
    if fail_on_error {
        assert!(matches!(outcome, Err(HookError::Field(_))));
    } else {
        outcome.expect("fail open");
    }
}

#[test]
fn error_handling_converts_a_scan_block() {
    let scan = ScanError::<ArmorTokenError>::Blocked {
        status: 400,
        detail: IndexMap::from([("error".to_owned(), text("Content blocked by Model Armor"))]),
    };
    let (status, detail) = blocked(HookError::from(scan));
    assert_eq!(status, 400);
    assert_eq!(
        detail.get("error"),
        Some(&text("Content blocked by Model Armor"))
    );
}

#[test]
fn error_handling_converts_scan_api_and_transport_errors() {
    let api = ScanError::<ArmorTokenError>::Api(ModelArmorAPIError {
        detail: "Model Armor API error (upstream 400)".to_owned(),
    });
    assert!(matches!(HookError::from(api), HookError::Api(_)));
    let transport = ScanError::<ArmorTokenError>::Transport(TransportError::MissingTemplate);
    assert!(matches!(
        HookError::from(transport),
        HookError::Transport(TransportError::MissingTemplate)
    ));
}
