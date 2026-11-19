#![cfg(test)]

use crate::limiter_records::ReservationBackend;
use crate::post_call_settlement::success_event::SuccessCallback;
use crate::post_call_settlement::tests::support::{
    RerankIo, SettlementFixture, SuccessFixture, WINDOW, install, operation_tuples,
    pipeline_tuples, scopes, window_identity,
};

#[tokio::test]
async fn applies_an_unguarded_project_reconciliation_through_a_pipeline() {
    let fixture = SettlementFixture::default();
    let success = SuccessFixture::default();
    install(&success.slot, |state| {
        state.itpm_reserved_tokens = 500;
        state.itpm_reserved_scopes = scopes(&[("project", "p1")]);
    });
    fixture
        .settlement()
        .log_success_event(&SuccessCallback {
            event: success.event(),
            io: RerankIo {
                tokens: Some((100, 0)),
                ..RerankIo::default()
            }
            .reconciliation(),
        })
        .await;
    assert_eq!(
        pipeline_tuples(&fixture.usage.pipelines()),
        vec![vec![(
            "{project:p1}:tokens".to_owned(),
            (-400.0_f64).to_bits(),
            Some(WINDOW),
        )]]
    );
    assert!(fixture.usage.increments().is_empty());
}

#[tokio::test]
async fn applies_a_windowed_project_reconciliation_reservation_aware() {
    let fixture = SettlementFixture::default();
    let success = SuccessFixture::default();
    install(&success.slot, |state| {
        state.itpm_reserved_tokens = 100;
        state.itpm_reserved_scopes = scopes(&[("project", "p1")]);
        state.itpm_reserved_windows.insert(window_identity(
            "{project:p1}:tokens",
            ReservationBackend::Redis,
        ));
    });
    fixture
        .settlement()
        .log_success_event(&SuccessCallback {
            event: success.event(),
            io: RerankIo {
                tokens: Some((250, 0)),
                ..RerankIo::default()
            }
            .reconciliation(),
        })
        .await;
    assert_eq!(
        operation_tuples(&fixture.usage.increments()),
        vec![(
            "{project:p1}:tokens".to_owned(),
            150.0_f64.to_bits(),
            Some(WINDOW),
        )]
    );
    assert!(fixture.usage.pipelines().is_empty());
}

#[tokio::test]
async fn settles_a_released_project_reservation_at_the_actual_usage() {
    let fixture = SettlementFixture::default();
    let success = SuccessFixture::default();
    install(&success.slot, |state| {
        state.itpm_reserved_tokens = 500;
        state.itpm_reserved_scopes = scopes(&[("project", "p1")]);
        state.reservation_released = true;
    });
    fixture
        .settlement()
        .log_success_event(&SuccessCallback {
            event: success.event(),
            io: RerankIo {
                tokens: Some((100, 0)),
                ..RerankIo::default()
            }
            .reconciliation(),
        })
        .await;
    assert_eq!(
        pipeline_tuples(&fixture.usage.pipelines()),
        vec![vec![(
            "{project:p1}:tokens".to_owned(),
            100.0_f64.to_bits(),
            Some(WINDOW),
        )]]
    );
}

#[tokio::test]
async fn writes_nothing_without_project_reservations() {
    let fixture = SettlementFixture::default();
    let success = SuccessFixture::default();
    install(&success.slot, |_state| {});
    fixture
        .settlement()
        .log_success_event(&SuccessCallback {
            event: success.event(),
            io: RerankIo {
                tokens: Some((250, 0)),
                ..RerankIo::default()
            }
            .reconciliation(),
        })
        .await;
    assert!(fixture.usage.increments().is_empty());
    assert!(fixture.usage.pipelines().is_empty());
}
