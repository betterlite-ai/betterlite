#![cfg(test)]

use std::collections::BTreeSet;

use crate::limiter_records::{ReservationBackend, ReservationWindowIdentity};
use crate::token_adjustments::builders::{
    TokenReconciliation, build_project_reservation_op, build_project_reservation_ops,
    build_tpm_reconciliation_ops,
};
use crate::token_adjustments::store::TokenAdjustmentError;

use super::support::FIRST_INEXACT_COUNT;

fn scopes(scopes: &[(&str, &str)]) -> BTreeSet<(String, String)> {
    scopes
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

fn identity(
    counter_key: &str,
    window_start: &str,
    backend: ReservationBackend,
) -> ReservationWindowIdentity {
    ReservationWindowIdentity {
        counter_key: counter_key.to_owned(),
        window_start: window_start.to_owned(),
        backend,
    }
}

fn identities(identity: &ReservationWindowIdentity) -> BTreeSet<ReservationWindowIdentity> {
    let mut set = BTreeSet::new();
    set.insert(identity.clone());
    set
}

#[test]
fn an_unreserved_target_gets_the_whole_actual_count() {
    let reserved = scopes(&[]);
    let reconciliation = TokenReconciliation {
        reserved_scopes: &reserved,
        actual_tokens: 10,
        reserved_tokens: 4,
        window_size: 60,
    };
    let targets = vec![("user".to_owned(), "u1".to_owned())];
    let operations =
        build_tpm_reconciliation_ops(&targets, &reconciliation).expect("the unreserved delta");
    assert_eq!(
        super::support::operation_tuples(&operations),
        vec![("{user:u1}:tokens".to_owned(), 10.0, Some(60))]
    );
}

#[test]
fn a_reserved_target_gets_the_difference() {
    let reserved = scopes(&[("user", "u1")]);
    let reconciliation = TokenReconciliation {
        reserved_scopes: &reserved,
        actual_tokens: 10,
        reserved_tokens: 4,
        window_size: 60,
    };
    let targets = vec![("user".to_owned(), "u1".to_owned())];
    let operations =
        build_tpm_reconciliation_ops(&targets, &reconciliation).expect("the reserved delta");
    assert_eq!(
        super::support::operation_tuples(&operations),
        vec![("{user:u1}:tokens".to_owned(), 6.0, Some(60))]
    );
}

#[test]
fn a_reserved_target_that_spent_what_it_reserved_gets_no_operation() {
    let reserved = scopes(&[("user", "u1")]);
    let reconciliation = TokenReconciliation {
        reserved_scopes: &reserved,
        actual_tokens: 4,
        reserved_tokens: 4,
        window_size: 60,
    };
    let targets = vec![("user".to_owned(), "u1".to_owned())];
    assert!(
        build_tpm_reconciliation_ops(&targets, &reconciliation)
            .expect("the empty delta")
            .is_empty()
    );
}

#[test]
fn an_unreserved_target_that_spent_nothing_gets_no_operation() {
    let reserved = scopes(&[]);
    let reconciliation = TokenReconciliation {
        reserved_scopes: &reserved,
        actual_tokens: 0,
        reserved_tokens: 4,
        window_size: 60,
    };
    let targets = vec![("user".to_owned(), "u1".to_owned())];
    assert!(
        build_tpm_reconciliation_ops(&targets, &reconciliation)
            .expect("the empty delta")
            .is_empty()
    );
}

#[test]
fn a_refund_under_a_reserved_scope_stays_negative() {
    let reserved = scopes(&[("user", "u1")]);
    let reconciliation = TokenReconciliation {
        reserved_scopes: &reserved,
        actual_tokens: 2,
        reserved_tokens: 5,
        window_size: 60,
    };
    let targets = vec![("user".to_owned(), "u1".to_owned())];
    let operations = build_tpm_reconciliation_ops(&targets, &reconciliation).expect("the refund");
    assert_eq!(
        super::support::operation_tuples(&operations),
        vec![("{user:u1}:tokens".to_owned(), -3.0, Some(60))]
    );
}

#[test]
fn each_target_keeps_its_scope_and_the_window_size() {
    let reserved = scopes(&[("user", "u1")]);
    let reconciliation = TokenReconciliation {
        reserved_scopes: &reserved,
        actual_tokens: 10,
        reserved_tokens: 4,
        window_size: 90,
    };
    let targets = vec![
        ("user".to_owned(), "u1".to_owned()),
        ("team".to_owned(), "t1".to_owned()),
    ];
    let operations = build_tpm_reconciliation_ops(&targets, &reconciliation).expect("both deltas");
    assert_eq!(
        super::support::operation_tuples(&operations),
        vec![
            ("{user:u1}:tokens".to_owned(), 6.0, Some(90)),
            ("{team:t1}:tokens".to_owned(), 10.0, Some(90)),
        ]
    );
}

#[test]
fn a_token_count_beyond_the_exact_float_range_is_an_error() {
    let reserved = scopes(&[]);
    let reconciliation = TokenReconciliation {
        reserved_scopes: &reserved,
        actual_tokens: FIRST_INEXACT_COUNT,
        reserved_tokens: 0,
        window_size: 60,
    };
    let targets = vec![("user".to_owned(), "u1".to_owned())];
    let result = build_tpm_reconciliation_ops(&targets, &reconciliation);
    assert!(matches!(
        result,
        Err(TokenAdjustmentError::TokenAmount {
            value: FIRST_INEXACT_COUNT
        })
    ));
}

#[test]
fn a_reserved_scope_with_an_identity_gets_the_window_guard() {
    let reserved = scopes(&[("user", "u1")]);
    let reconciliation = TokenReconciliation {
        reserved_scopes: &reserved,
        actual_tokens: 10,
        reserved_tokens: 4,
        window_size: 60,
    };
    let window_identities = identities(&identity(
        "{user:u1}:tokens",
        "100",
        ReservationBackend::Redis,
    ));
    let target = ("user".to_owned(), "u1".to_owned());
    let reservation = build_project_reservation_op(&target, &reconciliation, &window_identities)
        .expect("the reserved project operation")
        .expect("the operation");
    assert_eq!(reservation.operation.key, "{user:u1}:tokens");
    assert_eq!(reservation.operation.increment_value, 6.0);
    assert_eq!(reservation.operation.ttl, Some(60));
    assert_eq!(reservation.window_key.as_deref(), Some("{user:u1}:window"));
    assert_eq!(reservation.expected_window_start.as_deref(), Some("100"));
    assert_eq!(
        reservation.reservation_backend,
        Some(ReservationBackend::Redis)
    );
}

#[test]
fn a_reserved_scope_without_an_identity_is_a_plain_increment() {
    let reserved = scopes(&[("user", "u1")]);
    let reconciliation = TokenReconciliation {
        reserved_scopes: &reserved,
        actual_tokens: 10,
        reserved_tokens: 4,
        window_size: 60,
    };
    let window_identities = BTreeSet::new();
    let target = ("user".to_owned(), "u1".to_owned());
    let reservation = build_project_reservation_op(&target, &reconciliation, &window_identities)
        .expect("the reserved project operation")
        .expect("the operation");
    assert_eq!(reservation.operation.key, "{user:u1}:tokens");
    assert_eq!(reservation.operation.increment_value, 6.0);
    assert!(reservation.window_key.is_none());
    assert!(reservation.expected_window_start.is_none());
    assert!(reservation.reservation_backend.is_none());
}

#[test]
fn an_unreserved_scope_ignores_a_window_identity() {
    let reserved = scopes(&[]);
    let reconciliation = TokenReconciliation {
        reserved_scopes: &reserved,
        actual_tokens: 10,
        reserved_tokens: 4,
        window_size: 60,
    };
    let window_identities = identities(&identity(
        "{user:u1}:tokens",
        "100",
        ReservationBackend::Local,
    ));
    let target = ("user".to_owned(), "u1".to_owned());
    let reservation = build_project_reservation_op(&target, &reconciliation, &window_identities)
        .expect("the unreserved project operation")
        .expect("the operation");
    assert_eq!(reservation.operation.increment_value, 10.0);
    assert!(reservation.window_key.is_none());
    assert!(reservation.reservation_backend.is_none());
}

#[test]
fn a_window_identity_for_another_counter_is_not_used() {
    let reserved = scopes(&[("user", "u1")]);
    let reconciliation = TokenReconciliation {
        reserved_scopes: &reserved,
        actual_tokens: 10,
        reserved_tokens: 4,
        window_size: 60,
    };
    let window_identities = identities(&identity(
        "{user:u2}:tokens",
        "100",
        ReservationBackend::Local,
    ));
    let target = ("user".to_owned(), "u1".to_owned());
    let reservation = build_project_reservation_op(&target, &reconciliation, &window_identities)
        .expect("the reserved project operation")
        .expect("the operation");
    assert!(reservation.window_key.is_none());
    assert!(reservation.reservation_backend.is_none());
}

#[test]
fn a_reserved_scope_that_spent_what_it_reserved_has_no_operation() {
    let reserved = scopes(&[("user", "u1")]);
    let reconciliation = TokenReconciliation {
        reserved_scopes: &reserved,
        actual_tokens: 4,
        reserved_tokens: 4,
        window_size: 60,
    };
    let window_identities = BTreeSet::new();
    let target = ("user".to_owned(), "u1".to_owned());
    assert!(
        build_project_reservation_op(&target, &reconciliation, &window_identities)
            .expect("the empty delta")
            .is_none()
    );
}

#[test]
fn the_plural_builder_keeps_target_order() {
    let reserved = scopes(&[("user", "u1"), ("user", "u2")]);
    let reconciliation = TokenReconciliation {
        reserved_scopes: &reserved,
        actual_tokens: 10,
        reserved_tokens: 4,
        window_size: 60,
    };
    let window_identities = BTreeSet::new();
    let targets = vec![
        ("user".to_owned(), "u1".to_owned()),
        ("team".to_owned(), "t1".to_owned()),
        ("user".to_owned(), "u2".to_owned()),
    ];
    let reservations = build_project_reservation_ops(&targets, &reconciliation, &window_identities)
        .expect("the project operations");
    assert_eq!(reservations.len(), 3);
    assert_eq!(reservations[0].operation.key, "{user:u1}:tokens");
    assert_eq!(reservations[0].operation.increment_value, 6.0);
    assert_eq!(reservations[1].operation.key, "{team:t1}:tokens");
    assert_eq!(reservations[1].operation.increment_value, 10.0);
    assert_eq!(reservations[2].operation.key, "{user:u2}:tokens");
    assert_eq!(reservations[2].operation.increment_value, 6.0);
}

#[test]
fn the_plural_builder_drops_an_empty_delta() {
    let reserved = scopes(&[("user", "u1"), ("user", "u2")]);
    let reconciliation = TokenReconciliation {
        reserved_scopes: &reserved,
        actual_tokens: 4,
        reserved_tokens: 4,
        window_size: 60,
    };
    let window_identities = BTreeSet::new();
    let targets = vec![
        ("user".to_owned(), "u1".to_owned()),
        ("team".to_owned(), "t1".to_owned()),
        ("user".to_owned(), "u2".to_owned()),
    ];
    let reservations = build_project_reservation_ops(&targets, &reconciliation, &window_identities)
        .expect("the project operations");
    assert_eq!(reservations.len(), 1);
    assert_eq!(reservations[0].operation.key, "{team:t1}:tokens");
    assert_eq!(reservations[0].operation.increment_value, 4.0);
}

#[test]
fn the_plural_builder_keeps_a_negative_refund() {
    let reserved = scopes(&[("user", "u1")]);
    let reconciliation = TokenReconciliation {
        reserved_scopes: &reserved,
        actual_tokens: 1,
        reserved_tokens: 4,
        window_size: 60,
    };
    let window_identities = BTreeSet::new();
    let targets = vec![("user".to_owned(), "u1".to_owned())];
    let reservations = build_project_reservation_ops(&targets, &reconciliation, &window_identities)
        .expect("the refund");
    assert_eq!(reservations.len(), 1);
    assert_eq!(reservations[0].operation.increment_value, -3.0);
}
