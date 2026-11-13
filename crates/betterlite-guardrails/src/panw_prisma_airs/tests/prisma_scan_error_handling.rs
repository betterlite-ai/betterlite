use std::time::{Duration, SystemTime};

use betterlite_config::sections::panw_prisma_airs::FallbackAction;
use rstest::rstest;

use super::support::{guardrail, settings};
use crate::panw_prisma_airs::audit::{Direction, GuardrailAudit, GuardrailStatus, Stage};
use crate::panw_prisma_airs::error_detail::{ErrorCode, ErrorType, GuardrailHttpError};
use crate::panw_prisma_airs::error_handling::handle_api_error_with_logging;
use crate::panw_prisma_airs::verdict::{Action, Failure, ScanVerdict};

fn start() -> SystemTime {
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(2))
        .expect("time in range")
}

fn failed(category: &str, failure: Option<Failure>) -> ScanVerdict {
    ScanVerdict::synthetic(Action::Block, category, failure)
}

fn handle(
    fallback: FallbackAction,
    verdict: &ScanVerdict,
    direction: Direction,
) -> (Result<(), GuardrailHttpError>, GuardrailAudit) {
    let mut config = settings("http://unused.test");
    config.fallback_on_error = fallback;
    let mut audit = GuardrailAudit::default();
    let outcome =
        handle_api_error_with_logging(&guardrail(config), verdict, &mut audit, start(), direction);
    (outcome, audit)
}

#[test]
fn a_config_error_is_a_500_configuration_failure() {
    let (outcome, _) = handle(
        FallbackAction::Block,
        &failed("config_error", Some(Failure::Permanent)),
        Direction::Request,
    );
    let error = outcome.expect_err("must block");
    assert_eq!(error.status, 500);
    assert_eq!(
        serde_json::to_string(&error.detail).expect("detail"),
        r#"{"error":{"message":"Security scan failed - configuration error","type":"guardrail_config_error","code":"panw_prisma_airs_config_error","guardrail":"panw","category":"config_error"}}"#
    );
}

#[test]
fn any_other_permanent_failure_is_a_500_scan_failure_even_with_fallback_allow() {
    let (outcome, _) = handle(
        FallbackAction::Allow,
        &failed("http_404_error", Some(Failure::Permanent)),
        Direction::Request,
    );
    let error = outcome.expect_err("must block");
    assert_eq!(error.status, 500);
    assert_eq!(error.detail.error.kind, ErrorType::GuardrailScanError);
    assert_eq!(error.detail.error.code, ErrorCode::PanwPrismaAirsScanFailed);
    assert_eq!(error.detail.error.category, "http_404_error");
    assert_eq!(
        error.detail.error.message,
        "Security scan failed - request blocked for safety"
    );
}

#[rstest]
#[case::request(Direction::Request)]
#[case::response(Direction::Response)]
fn a_transient_failure_with_fallback_allow_lets_the_call_through_unscanned(
    #[case] direction: Direction,
) {
    let (outcome, audit) = handle(
        FallbackAction::Allow,
        &failed("timeout_error", Some(Failure::Transient)),
        direction,
    );
    outcome.expect("fallback allows");
    assert_eq!(audit.applied_guardrails, ["panw:unscanned"]);
}

#[test]
fn a_transient_failure_with_fallback_block_is_a_500_scan_failure() {
    let (outcome, audit) = handle(
        FallbackAction::Block,
        &failed("network_error", Some(Failure::Transient)),
        Direction::Request,
    );
    let error = outcome.expect_err("must block");
    assert_eq!(error.status, 500);
    assert_eq!(error.detail.error.code, ErrorCode::PanwPrismaAirsScanFailed);
    assert_eq!(error.detail.error.category, "network_error");
    assert!(audit.applied_guardrails.is_empty());
}

#[test]
fn a_failure_without_a_flag_is_a_500_scan_failure_even_with_fallback_allow() {
    let (outcome, audit) = handle(
        FallbackAction::Allow,
        &failed("api_error", None),
        Direction::Request,
    );
    assert_eq!(outcome.expect_err("must block").status, 500);
    assert!(audit.applied_guardrails.is_empty());
}

#[test]
fn the_failure_is_logged_with_the_injected_clock() {
    let (_, audit) = handle(
        FallbackAction::Block,
        &failed("timeout_error", Some(Failure::Transient)),
        Direction::Response,
    );
    let [entry] = audit.information.as_slice() else {
        panic!("expected one entry, got {}", audit.information.len());
    };
    assert_eq!(entry.provider, "panw_prisma_airs");
    assert_eq!(entry.status, GuardrailStatus::GuardrailFailedToRespond);
    assert_eq!(entry.event_type, Stage::PostCall);
    assert_eq!(entry.start_time, start());
    assert_eq!(
        entry.end_time,
        SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_secs(12))
            .expect("time in range")
    );
    assert_eq!(entry.duration, Duration::from_secs(10));
    assert_eq!(entry.response.category.as_deref(), Some("timeout_error"));
    assert_eq!(entry.response.failure, Some(Failure::Transient));
}

#[test]
fn the_failure_is_logged_before_a_permanent_failure_blocks() {
    let (outcome, audit) = handle(
        FallbackAction::Block,
        &failed("config_error", Some(Failure::Permanent)),
        Direction::Request,
    );
    outcome.expect_err("must block");
    assert_eq!(audit.information.len(), 1);
    assert_eq!(audit.information[0].event_type, Stage::PreCall);
}
