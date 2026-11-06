use axum::http::StatusCode;
use rstest::rstest;

use super::support::{guardrail, settings};
use crate::panw_prisma_airs::audit::{Direction, GuardrailAudit, ScanIdEntry, Stage};
use crate::panw_prisma_airs::error_handling::record_scan_id;
use crate::panw_prisma_airs::scan::call_panw_api;
use crate::panw_prisma_airs::verdict::{Action, ScanVerdict};

fn verdict_with(scan_id: Option<&str>) -> ScanVerdict {
    let mut verdict = ScanVerdict::synthetic(Action::Allow, "benign", None);
    verdict.scan_id = scan_id.map(str::to_owned);
    verdict
}

#[rstest]
#[case::request(Direction::Request, Stage::PreCall)]
#[case::response(Direction::Response, Stage::PostCall)]
fn the_scan_id_is_recorded_for_the_stage_of_its_direction(
    #[case] direction: Direction,
    #[case] stage: Stage,
) {
    let guardrail = guardrail(settings("http://unused.test"));
    let mut audit = GuardrailAudit::default();
    record_scan_id(
        &guardrail,
        &mut audit,
        &verdict_with(Some("scan-7")),
        direction,
    );
    assert_eq!(audit.scan_ids, ["scan-7"]);
    assert_eq!(
        audit.scan_metadata,
        [ScanIdEntry {
            guardrail: "panw".to_owned(),
            stage,
            provider: "panw_prisma_airs".to_owned(),
            scan_id: "scan-7".to_owned(),
        }]
    );
}

#[rstest]
#[case::missing(None)]
#[case::empty(Some(""))]
fn a_missing_or_empty_scan_id_records_nothing(#[case] scan_id: Option<&str>) {
    let guardrail = guardrail(settings("http://unused.test"));
    let mut audit = GuardrailAudit::default();
    record_scan_id(
        &guardrail,
        &mut audit,
        &verdict_with(scan_id),
        Direction::Request,
    );
    assert!(audit.scan_ids.is_empty());
    assert!(audit.scan_metadata.is_empty());
}

#[test]
fn the_same_scan_id_is_recorded_once_per_stage() {
    let guardrail = guardrail(settings("http://unused.test"));
    let mut audit = GuardrailAudit::default();
    let verdict = verdict_with(Some("scan-7"));
    record_scan_id(&guardrail, &mut audit, &verdict, Direction::Request);
    record_scan_id(&guardrail, &mut audit, &verdict, Direction::Request);
    record_scan_id(&guardrail, &mut audit, &verdict, Direction::Response);
    assert_eq!(audit.scan_ids, ["scan-7"]);
    assert_eq!(audit.scan_metadata.len(), 2);
}

#[tokio::test]
async fn a_scanned_verdict_carries_the_scan_id_that_gets_recorded() {
    let server = super::support::serve(
        StatusCode::OK,
        br#"{"action":"allow","category":"benign","scan_id":"from-api"}"#,
    )
    .await;
    let guardrail = guardrail(settings(&server.base));
    let verdict = call_panw_api(
        &guardrail,
        crate::panw_prisma_airs::payload::ScanRequest {
            content: "hello",
            direction: Direction::Request,
            metadata: None,
            call_id: Some("call-1"),
            tool_event: None,
        },
    )
    .await;
    let mut audit = GuardrailAudit::default();
    record_scan_id(&guardrail, &mut audit, &verdict, Direction::Request);
    assert_eq!(audit.scan_ids, ["from-api"]);
}
