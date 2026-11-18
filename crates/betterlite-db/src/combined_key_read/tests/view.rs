#![cfg(test)]

use chrono::DateTime;
use rust_decimal::Decimal;

use crate::budget_permission_rows::JsonData;
use crate::identity_rows::key::Expiry;
use crate::team_rows::member::MemberRole;

use super::super::view::{CombinedKeyView, ViewError, epoch_decimal};
use super::harness::{decimal, fixed_now, number, object, text, view_fields};

#[test]
fn last_matching_member_not_email() {
    let fields = view_fields(vec![
        ("user_id", text("u1")),
        (
            "team_members_with_roles",
            JsonData::Array(vec![
                object(vec![
                    ("user_id", text("u1")),
                    ("user_email", text("first@example.com")),
                    ("role", text("admin")),
                ]),
                object(vec![
                    ("user_id", JsonData::Null),
                    ("user_email", text("email-only@example.com")),
                    ("role", text("user")),
                ]),
                object(vec![
                    ("user_id", text("u1")),
                    ("user_email", text("last@example.com")),
                    ("role", text("user")),
                ]),
                object(vec![
                    ("user_id", text("u2")),
                    ("user_email", text("other@example.com")),
                    ("role", text("admin")),
                ]),
            ]),
        ),
    ]);
    let view = CombinedKeyView::construct(fields, epoch_decimal(fixed_now())).unwrap();
    let member = view.team_member.unwrap();
    assert_eq!(member.base.user_email.as_deref(), Some("last@example.com"));
    assert_eq!(member.role, MemberRole::User);
}

#[test]
fn normalized_team_fields_and_expiry() {
    normalizes_team_defaults();
    applies_budget_aliases();
    stringifies_end_user_and_promotes_org();
    formats_expiry_like_isoformat();
}

#[test]
fn null_token_columns_take_their_defaults() {
    let fields = view_fields(vec![
        ("max_budget", JsonData::Null),
        ("spend", JsonData::Null),
        ("expires", JsonData::Null),
        ("team_max_budget", JsonData::Null),
    ]);
    let view = CombinedKeyView::construct(fields, epoch_decimal(fixed_now())).unwrap();
    assert!(view.token.max_budget.is_none());
    assert_eq!(view.token.spend, Decimal::ZERO);
    assert!(view.token.expires.is_none());
    assert!(view.team_max_budget.is_none());
}

#[test]
fn epoch_decimal_whole_and_fractional_seconds() {
    assert_eq!(epoch_decimal(fixed_now()), Decimal::from(1_700_000_000_i64));
    let fractional = DateTime::from_timestamp(1_700_000_000, 250_000_000).unwrap();
    assert_eq!(epoch_decimal(fractional), Decimal::new(170_000_000_025, 2));
}

#[test]
fn null_budget_alias_never_clears_the_key_mapping() {
    let fields = view_fields(vec![
        ("model_max_budget", object(Vec::new())),
        ("litellm_budget_table_model_max_budget", JsonData::Null),
    ]);
    let view = CombinedKeyView::construct(fields, epoch_decimal(fixed_now())).unwrap();
    assert!(view.token.model_max_budget.0.is_empty());
}

#[test]
fn fractional_end_user_id_is_a_typed_error() {
    let fields = view_fields(vec![(
        "end_user_id",
        JsonData::Number(serde_json::Number::from_f64(1.5).unwrap()),
    )]);
    assert!(matches!(
        CombinedKeyView::construct(fields, epoch_decimal(fixed_now())),
        Err(ViewError::Json(_))
    ));
}

fn normalizes_team_defaults() {
    let fields = view_fields(vec![
        ("user_id", text("u1")),
        ("team_models", JsonData::Null),
        ("team_blocked", JsonData::Null),
        ("team_members_with_roles", JsonData::Array(Vec::new())),
    ]);
    let now = fixed_now();
    let view = CombinedKeyView::construct(fields, epoch_decimal(now)).unwrap();
    assert!(view.team_models.is_empty());
    assert!(!view.team_blocked);
    assert!(view.team_member.is_none());
    assert_eq!(view.last_refreshed_at, Some(epoch_decimal(now)));
}

fn applies_budget_aliases() {
    let fields = view_fields(vec![
        ("litellm_budget_table_max_budget", decimal("100")),
        ("litellm_budget_table_soft_budget", decimal("5")),
        ("litellm_budget_table_tpm_limit", number(21)),
        ("litellm_budget_table_rpm_limit", number(22)),
        ("litellm_budget_table_tpd_limit", number(23)),
        (
            "litellm_budget_table_model_max_budget",
            object(vec![("m", number(5))]),
        ),
        ("model_max_budget", object(Vec::new())),
        ("max_budget", decimal("7")),
    ]);
    let view = CombinedKeyView::construct(fields, epoch_decimal(fixed_now())).unwrap();
    assert_eq!(view.token.max_budget, Some(Decimal::from(7_i32)));
    assert_eq!(view.soft_budget, Some(Decimal::from(5_i32)));
    assert_eq!(view.token.tpm_limit, Some(21));
    assert_eq!(view.token.rpm_limit, Some(22));
    assert_eq!(view.token.tpd_limit, Some(23));
    assert_eq!(view.token.model_max_budget.0.get("m"), Some(&number(5)));
    let kept = view_fields(vec![
        (
            "litellm_budget_table_model_max_budget",
            object(vec![("b", number(1))]),
        ),
        ("model_max_budget", object(vec![("k", number(2))])),
    ]);
    let kept_view = CombinedKeyView::construct(kept, epoch_decimal(fixed_now())).unwrap();
    assert_eq!(
        kept_view.token.model_max_budget.0.get("k"),
        Some(&number(2))
    );
    assert!(!kept_view.token.model_max_budget.0.contains_key("b"));
}

fn stringifies_end_user_and_promotes_org() {
    let fields = view_fields(vec![
        ("end_user_id", number(42)),
        ("organization_id", text("org-key")),
        ("org_id", text("org-team")),
    ]);
    let view = CombinedKeyView::construct(fields, epoch_decimal(fixed_now())).unwrap();
    assert_eq!(view.end_user_id.as_deref(), Some("42"));
    assert_eq!(view.token.org_id.as_deref(), Some("org-key"));
    let flagged = view_fields(vec![("end_user_id", JsonData::Bool(true))]);
    let flagged_view = CombinedKeyView::construct(flagged, epoch_decimal(fixed_now())).unwrap();
    assert_eq!(flagged_view.end_user_id.as_deref(), Some("True"));
}

fn formats_expiry_like_isoformat() {
    let micros = view_fields(vec![("expires", text("2024-01-02T03:04:05.120000"))]);
    let view = CombinedKeyView::construct(micros, epoch_decimal(fixed_now())).unwrap();
    assert_eq!(
        view.token.expires,
        Some(Expiry::Text(String::from("2024-01-02T03:04:05.120000")))
    );
    let zero = view_fields(vec![("expires", text("2024-01-02T03:04:05.000000"))]);
    let zero_view = CombinedKeyView::construct(zero, epoch_decimal(fixed_now())).unwrap();
    assert_eq!(
        zero_view.token.expires,
        Some(Expiry::Text(String::from("2024-01-02T03:04:05")))
    );
    let offset = view_fields(vec![("expires", text("2024-01-02T03:04:05+02:00"))]);
    let offset_view = CombinedKeyView::construct(offset, epoch_decimal(fixed_now())).unwrap();
    assert_eq!(
        offset_view.token.expires,
        Some(Expiry::Text(String::from("2024-01-02T03:04:05+02:00")))
    );
}
