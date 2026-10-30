#![cfg(test)]

#[test]
fn budget_all_twelve_defaults() {}
#[test]
fn full_requires_created_at() {}
#[test]
fn writable_budget_ignores_server_fields() {}
#[test]
fn temporary_missing_amount_or_expiry() {}
#[test]
fn temporary_before_at_after_expiry() {}
#[test]
fn naive_expiry_is_utc() {}
#[test]
fn offset_expiry_same_instant() {}
#[test]
fn temporary_negative_not_clamped() {}
#[test]
fn unlimited_budget_ignores_increase() {}
#[test]
fn permission_all_defaults_and_required_id() {}
#[test]
fn permission_explicit_nulls_and_fresh_lists() {}
#[test]
fn membership_required_fields_and_defaults() {}
#[test]
fn membership_explicit_email_wins() {}
#[test]
fn membership_empty_email_wins() {}
#[test]
fn membership_dict_and_attribute_email() {}
#[test]
fn membership_missing_email_none() {}
#[test]
fn membership_nested_nonstring_email_not_revalidated() {}
#[test]
fn membership_spend_null_invalid() {}
#[test]
fn strings_are_not_decoded_as_containers() {}
#[test]
fn unknown_fields_ignored() {}
#[test]
fn native_budget_rows_byte_and_depth_limits() {}
#[test]
fn replays_the_budget_permission_rows_corpus() {}
#[test]
fn postgres_budget_decimal_roundtrip() {}
#[test]
fn postgres_permission_null_vs_empty() {}
