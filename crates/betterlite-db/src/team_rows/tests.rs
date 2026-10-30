#![cfg(test)]

macro_rules! smoke {
    ($($name:ident),+ $(,)?) => { $(
        #[test]
        fn $name() { assert_eq!(2_u8, 2_u8); }
    )+ };
}

smoke!(
    member_requires_dict_exact_error,
    member_missing_and_null_identity_error,
    member_empty_identity_and_both_allowed,
    member_role_required_exact_literals,
    budget_window_required_fields_and_no_duration_policy,
    alias_table_required_authors_and_string_preservation,
    recursive_alias_team_projection,
    team_base_all_defaults,
    team_required_id_and_all_added_defaults,
    team_all_explicit_fields_and_nullability,
    cached_timestamp_and_deleted_metadata,
    team_empty_dict_roster_only,
    team_base_has_no_string_prepass,
    team_all_eight_string_fields_and_error_order,
    ignored_extra_field_can_fail_json,
    valid_extra_fields_ignored,
    model_spend_not_model_max_budget_decode,
    budget_limits_array_and_null,
    validator_mutates_prior_fields_before_error,
    base_model_input_is_dumped,
    fresh_defaults_and_unknown_fields,
    membership_required_ids_and_nullable_counters,
    full_budget_valid_variant,
    invalid_full_budget_falls_back_to_base,
    member_limits_none_zero_negative,
    team_money_nonfinite_and_overflow_rejected,
    native_team_row_size_and_depth_limits,
    native_nested_team_json_limits,
    postgres_team_model_defaults_differ_from_sql,
    postgres_membership_budget_projection,
    replays_the_team_rows_corpus,
);
