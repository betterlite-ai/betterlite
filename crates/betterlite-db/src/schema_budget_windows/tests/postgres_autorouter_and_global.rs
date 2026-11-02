#[derive(Clone, Copy)]
enum AutorouterAndGlobalCohort {
    SessionClassifierAndBaselineDefaults,
    SavingsEstimateCoverageDefaults,
    UserSessionAllCounterJsonAndSpendDefaults,
    UserSessionKeyAndTwoIndexes,
    GlobalTenBigintFiveSpendDefaults,
    GlobalUniqueTupleNullSemantics,
    IssuerEmptyBackfillAndNullRejection,
    IssuerDistinctClaimPairsInactiveStillUnique,
    BaselineRequiredInitialEquivalence,
    BaselineZeroRevisionsFalseRetired,
    DirtyIndexExcludesRetiredAndPublished,
    ObservationTextDataFalseConflictAndOrderIndexes,
}

fn autorouter_and_global_checkpoint_for(cohort: AutorouterAndGlobalCohort) -> &'static str {
    match cohort {
        AutorouterAndGlobalCohort::SessionClassifierAndBaselineDefaults => {
            ADD_AUTOROUTER_SESSION_BASELINE_MODELS.directory
        }
        AutorouterAndGlobalCohort::SavingsEstimateCoverageDefaults => {
            ADD_AUTOROUTER_SAVINGS_ESTIMATE_COVERAGE.directory
        }
        AutorouterAndGlobalCohort::UserSessionAllCounterJsonAndSpendDefaults
        | AutorouterAndGlobalCohort::UserSessionKeyAndTwoIndexes => {
            ADD_AUTOROUTER_USER_SESSION_ROLLUP.directory
        }
        AutorouterAndGlobalCohort::GlobalTenBigintFiveSpendDefaults
        | AutorouterAndGlobalCohort::GlobalUniqueTupleNullSemantics => {
            ADD_DAILY_GLOBAL_SPEND.directory
        }
        AutorouterAndGlobalCohort::IssuerEmptyBackfillAndNullRejection => {
            ADD_DAILY_RESPONSE_TIME.directory
        }
        AutorouterAndGlobalCohort::IssuerDistinctClaimPairsInactiveStillUnique => {
            SCOPE_JWT_KEY_MAPPING_BY_ISSUER.directory
        }
        AutorouterAndGlobalCohort::BaselineRequiredInitialEquivalence
        | AutorouterAndGlobalCohort::BaselineZeroRevisionsFalseRetired
        | AutorouterAndGlobalCohort::DirtyIndexExcludesRetiredAndPublished
        | AutorouterAndGlobalCohort::ObservationTextDataFalseConflictAndOrderIndexes => {
            ADD_AUTOROUTER_BASELINE_STATE.directory
        }
    }
}

async fn insert_autorouter_session(client: &Client, key: &str) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_AutoRouterSession" (api_key, session_id, router_name, router_type, first_turn_at, last_turn_at, last_model) VALUES ($1, 'session-a', 'router-a', 'classifier', now(), now(), 'model-a')"#,
        &[&key],
    )
    .await;
}

async fn check_session_classifier_and_baseline_defaults(client: &Client) {
    insert_autorouter_session(client, "key-a").await;
    assert_columns(
        client,
        "LiteLLM_AutoRouterSession",
        &[
            float8_default("classifier_cost", "0"),
            int4_zero("classifier_cost_recorded_turns"),
            empty_jsonb("baseline_models"),
        ],
    )
    .await;
}

async fn check_savings_estimate_coverage_defaults(client: &Client) {
    insert_autorouter_session(client, "key-a").await;
    assert_columns(
        client,
        "LiteLLM_AutoRouterSession",
        &[
            int4_zero("savings_estimated_turns"),
            float8_default("savings_estimated_actual_spend", "0"),
            float8_default("savings_estimated_saved_spend", "0"),
            empty_jsonb("savings_estimated_baseline_models"),
        ],
    )
    .await;
}

async fn insert_autorouter_user_session(client: &Client, user_id: &str, api_key: &str) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_AutoRouterUserSession" (user_id, api_key, session_id, router_name, router_type, first_turn_at, last_turn_at, last_model) VALUES ($1, $2, 'session-a', 'router-a', 'classifier', now(), now(), 'model-a')"#,
        &[&user_id, &api_key],
    )
    .await;
}

async fn check_user_session_all_counter_json_and_spend_defaults(client: &Client) {
    insert_autorouter_user_session(client, "user-a", "key-a").await;
    assert_columns(
        client,
        "LiteLLM_AutoRouterUserSession",
        &[
            empty_jsonb("models"),
            int4_zero("turns"),
            int4_zero("unordered_turns"),
            int4_zero("covered_turns"),
            int4_zero("cache_hits"),
            int4_zero("same_model_turns"),
            int4_zero("same_model_hits"),
            int4_zero("first_visit_turns"),
            int4_zero("first_visit_hits"),
            int4_zero("return_turns"),
            int4_zero("return_hits"),
            int4_zero("return_expired_misses"),
            int4_zero("return_within_ttl_misses"),
            int4_zero("ttl_5m_turns"),
            int4_zero("ttl_1h_turns"),
            int8_zero("total_tokens"),
            float8_default("spend", "0"),
            float8_default("saved_spend", "0"),
            int4_zero("savings_estimated_turns"),
            float8_default("savings_estimated_actual_spend", "0"),
            float8_default("savings_estimated_saved_spend", "0"),
            empty_jsonb("savings_estimated_baseline_models"),
            float8_default("classifier_cost", "0"),
            int4_zero("classifier_cost_recorded_turns"),
            empty_jsonb("tier_turns"),
            empty_jsonb("baseline_models"),
        ],
    )
    .await;
}

async fn check_user_session_key_and_two_indexes(client: &Client) {
    assert_repeat_insert_unique_violation(
        client,
        r#"INSERT INTO "LiteLLM_AutoRouterUserSession" (user_id, api_key, session_id, router_name, router_type, first_turn_at, last_turn_at, last_model) VALUES ($1, $2, 'session-a', 'router-a', 'classifier', now(), now(), 'model-a')"#,
        &[&"user-a", &"key-a"],
        &[&"user-a", &"key-a"],
    )
    .await;
    assert_index(
        client,
        "idx_autorouter_user_session_last_turn",
        &["last_turn_at"],
        false,
    )
    .await;
    assert_index(
        client,
        "idx_autorouter_user_session_user_last_turn",
        &["user_id", "last_turn_at"],
        false,
    )
    .await;
}

type GlobalSpendScope<'scope> = (
    Option<&'scope str>,
    Option<&'scope str>,
    Option<&'scope str>,
    Option<&'scope str>,
    Option<&'scope str>,
);

const EMPTY_GLOBAL_SPEND_SCOPE: GlobalSpendScope<'static> = (None, None, None, None, None);

async fn insert_daily_global_spend(client: &Client, id: &str, scope: GlobalSpendScope<'_>) {
    let (model, model_group, custom_llm_provider, mcp_namespaced_tool_name, endpoint) = scope;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_DailyGlobalSpend" (id, date, model, model_group, custom_llm_provider, mcp_namespaced_tool_name, endpoint, updated_at) VALUES ($1, 'date-a', $2, $3, $4, $5, $6, now())"#,
        &[
            &id,
            &model,
            &model_group,
            &custom_llm_provider,
            &mcp_namespaced_tool_name,
            &endpoint,
        ],
    )
    .await;
}

async fn check_global_ten_bigint_five_spend_defaults(client: &Client) {
    insert_daily_global_spend(client, "global-a", EMPTY_GLOBAL_SPEND_SCOPE).await;
    assert_columns(
        client,
        "LiteLLM_DailyGlobalSpend",
        &[
            int8_zero("prompt_tokens"),
            int8_zero("completion_tokens"),
            int8_zero("cache_read_input_tokens"),
            int8_zero("cache_creation_input_tokens"),
            int8_zero("compression_saved_tokens"),
            int8_zero("api_requests"),
            int8_zero("successful_requests"),
            int8_zero("failed_requests"),
            int8_zero("total_response_time_ms"),
            int8_zero("timed_requests"),
            float8_default("compression_savings_spend", "0.0"),
            float8_default("prompt_caching_savings_spend", "0.0"),
            float8_default("gateway_injected_caching_savings_spend", "0.0"),
            float8_default("autorouter_savings_spend", "0.0"),
            float8_default("spend", "0.0"),
        ],
    )
    .await;
}

async fn check_global_unique_tuple_null_semantics(client: &Client) {
    insert_daily_global_spend(client, "global-null-1", EMPTY_GLOBAL_SPEND_SCOPE).await;
    insert_daily_global_spend(client, "global-null-2", EMPTY_GLOBAL_SPEND_SCOPE).await;
    insert_daily_global_spend(
        client,
        "global-full-1",
        (
            Some("model-a"),
            Some("group-a"),
            Some("provider-a"),
            Some("tool-a"),
            Some("endpoint-a"),
        ),
    )
    .await;
    assert_exec_params_err_state(
        client,
        r#"INSERT INTO "LiteLLM_DailyGlobalSpend" (id, date, model, model_group, custom_llm_provider, mcp_namespaced_tool_name, endpoint, updated_at) VALUES ($1, 'date-a', $2, $3, $4, $5, $6, now())"#,
        &[
            &"global-full-2",
            &"model-a",
            &"group-a",
            &"provider-a",
            &"tool-a",
            &"endpoint-a",
        ],
        SqlState::UNIQUE_VIOLATION,
    )
    .await;
}

async fn check_issuer_empty_backfill_and_null_rejection(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_VerificationToken" (token) VALUES ($1)"#,
        &[&"token-issuer"],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_JWTKeyMapping" (id, jwt_claim_name, jwt_claim_value, token) VALUES ($1, $2, $3, $4)"#,
        &[&"mapping-issuer", &"claim-a", &"value-a", &"token-issuer"],
    )
    .await;
    batch_execute(
        client,
        migration_sql_of(SCOPE_JWT_KEY_MAPPING_BY_ISSUER.directory),
    )
    .await;
    let jwt_issuer = scalar_text(
        client,
        r#"SELECT jwt_issuer FROM "LiteLLM_JWTKeyMapping" WHERE id = $1"#,
        &[&"mapping-issuer"],
    )
    .await;
    assert_eq!(jwt_issuer, "");
    assert_exec_params_err_state(
        client,
        r#"INSERT INTO "LiteLLM_JWTKeyMapping" (id, jwt_claim_name, jwt_claim_value, token, jwt_issuer) VALUES ($1, $2, $3, $4, NULL)"#,
        &[&"mapping-null-issuer", &"claim-b", &"value-b", &"token-issuer"],
        SqlState::NOT_NULL_VIOLATION,
    )
    .await;
}

async fn check_issuer_distinct_claim_pairs_inactive_still_unique(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_VerificationToken" (token) VALUES ($1)"#,
        &[&"token-distinct"],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_JWTKeyMapping" (id, jwt_claim_name, jwt_claim_value, token, is_active) VALUES ($1, $2, $3, $4, true)"#,
        &[&"mapping-active", &"claim-a", &"value-a", &"token-distinct"],
    )
    .await;
    assert_exec_params_err_state(
        client,
        r#"INSERT INTO "LiteLLM_JWTKeyMapping" (id, jwt_claim_name, jwt_claim_value, token, is_active) VALUES ($1, $2, $3, $4, false)"#,
        &[&"mapping-inactive", &"claim-a", &"value-a", &"token-distinct"],
        SqlState::UNIQUE_VIOLATION,
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_JWTKeyMapping" (id, jwt_claim_name, jwt_claim_value, token) VALUES ($1, $2, $3, $4)"#,
        &[&"mapping-distinct-value", &"claim-a", &"value-b", &"token-distinct"],
    )
    .await;
}

async fn insert_autorouter_baseline(client: &Client, scope: &str) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_AutoRouterBaselineComparison" (scope, api_key, session_id, router_name, initial_equivalent) VALUES ($1, 'key-a', 'session-a', 'router-a', true)"#,
        &[&scope],
    )
    .await;
}

async fn check_baseline_required_initial_equivalence(client: &Client) {
    insert_autorouter_baseline(client, "scope-a").await;
    assert_exec_params_err_state(
        client,
        r#"INSERT INTO "LiteLLM_AutoRouterBaselineComparison" (scope, api_key, session_id, router_name) VALUES ($1, 'key-b', 'session-b', 'router-b')"#,
        &[&"scope-b"],
        SqlState::NOT_NULL_VIOLATION,
    )
    .await;
    assert_batch_execute_err_state(
        client,
        r#"INSERT INTO "LiteLLM_AutoRouterBaselineComparison" (scope, api_key, session_id, router_name, initial_equivalent) VALUES ('scope-a', 'key-a', 'session-a', 'router-a', true)"#,
        SqlState::UNIQUE_VIOLATION,
    )
    .await;
}

async fn check_baseline_zero_revisions_false_retired(client: &Client) {
    insert_autorouter_baseline(client, "scope-a").await;
    assert_columns(
        client,
        "LiteLLM_AutoRouterBaselineComparison",
        &[
            int8_zero("revision"),
            int8_zero("published_revision"),
            bool_false("retired"),
        ],
    )
    .await;
}

async fn check_dirty_index_excludes_retired_and_published(client: &Client) {
    assert!(index_exists(client, "idx_autorouter_baseline_dirty").await);
    let predicate = index_predicate(client, "idx_autorouter_baseline_dirty")
        .await
        .unwrap();
    assert!(predicate.contains("retired"));
    assert!(predicate.contains("published_revision"));
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_AutoRouterBaselineComparison" (scope, api_key, session_id, router_name, initial_equivalent, revision, published_revision, retired) VALUES ($1, 'key-a', 'session-a', 'router-a', true, 1, 0, false)"#,
        &[&"scope-dirty"],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_AutoRouterBaselineComparison" (scope, api_key, session_id, router_name, initial_equivalent, revision, published_revision, retired) VALUES ($1, 'key-a', 'session-a', 'router-a', true, 1, 0, true)"#,
        &[&"scope-retired"],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_AutoRouterBaselineComparison" (scope, api_key, session_id, router_name, initial_equivalent, revision, published_revision, retired) VALUES ($1, 'key-a', 'session-a', 'router-a', true, 0, 0, false)"#,
        &[&"scope-clean"],
    )
    .await;
    let dirty_scopes = string_rows(
        client,
        r#"SELECT scope FROM "LiteLLM_AutoRouterBaselineComparison" WHERE NOT retired AND revision <> published_revision ORDER BY scope"#,
        &[],
    )
    .await;
    assert_eq!(dirty_scopes, vec!["scope-dirty".to_owned()]);
}

async fn check_observation_text_data_false_conflict_and_order_indexes(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_AutoRouterBaselineObservation" (request_id, scope, started_at, revision, data) VALUES ($1, 'scope-a', 1.0, 0, 'payload')"#,
        &[&"observation-a"],
    )
    .await;
    assert_column(
        client,
        "LiteLLM_AutoRouterBaselineObservation",
        bool_false("conflicted"),
    )
    .await;
    assert_exec_params_err_state(
        client,
        r#"INSERT INTO "LiteLLM_AutoRouterBaselineObservation" (request_id, scope, started_at, revision, data) VALUES ($1, 'scope-a', 2.0, 1, 'payload-again')"#,
        &[&"observation-a"],
        SqlState::UNIQUE_VIOLATION,
    )
    .await;
    assert_index(
        client,
        "idx_autorouter_baseline_event_order",
        &["scope", "started_at", "request_id"],
        false,
    )
    .await;
    assert_index(
        client,
        "idx_autorouter_baseline_event_revision",
        &["scope", "revision", "started_at"],
        false,
    )
    .await;
}

async fn run_autorouter_and_global_check(client: &Client, cohort: AutorouterAndGlobalCohort) {
    match cohort {
        AutorouterAndGlobalCohort::SessionClassifierAndBaselineDefaults => {
            check_session_classifier_and_baseline_defaults(client).await;
        }
        AutorouterAndGlobalCohort::SavingsEstimateCoverageDefaults => {
            check_savings_estimate_coverage_defaults(client).await;
        }
        AutorouterAndGlobalCohort::UserSessionAllCounterJsonAndSpendDefaults => {
            check_user_session_all_counter_json_and_spend_defaults(client).await;
        }
        AutorouterAndGlobalCohort::UserSessionKeyAndTwoIndexes => {
            check_user_session_key_and_two_indexes(client).await;
        }
        AutorouterAndGlobalCohort::GlobalTenBigintFiveSpendDefaults => {
            check_global_ten_bigint_five_spend_defaults(client).await;
        }
        AutorouterAndGlobalCohort::GlobalUniqueTupleNullSemantics => {
            check_global_unique_tuple_null_semantics(client).await;
        }
        AutorouterAndGlobalCohort::IssuerEmptyBackfillAndNullRejection => {
            check_issuer_empty_backfill_and_null_rejection(client).await;
        }
        AutorouterAndGlobalCohort::IssuerDistinctClaimPairsInactiveStillUnique => {
            check_issuer_distinct_claim_pairs_inactive_still_unique(client).await;
        }
        AutorouterAndGlobalCohort::BaselineRequiredInitialEquivalence => {
            check_baseline_required_initial_equivalence(client).await;
        }
        AutorouterAndGlobalCohort::BaselineZeroRevisionsFalseRetired => {
            check_baseline_zero_revisions_false_retired(client).await;
        }
        AutorouterAndGlobalCohort::DirtyIndexExcludesRetiredAndPublished => {
            check_dirty_index_excludes_retired_and_published(client).await;
        }
        AutorouterAndGlobalCohort::ObservationTextDataFalseConflictAndOrderIndexes => {
            check_observation_text_data_false_conflict_and_order_indexes(client).await;
        }
    }
}

#[rstest]
#[case::session_classifier_and_baseline_defaults(
    AutorouterAndGlobalCohort::SessionClassifierAndBaselineDefaults
)]
#[case::savings_estimate_coverage_defaults(
    AutorouterAndGlobalCohort::SavingsEstimateCoverageDefaults
)]
#[case::user_session_all_counter_json_and_spend_defaults(
    AutorouterAndGlobalCohort::UserSessionAllCounterJsonAndSpendDefaults
)]
#[case::user_session_key_and_two_indexes(AutorouterAndGlobalCohort::UserSessionKeyAndTwoIndexes)]
#[case::global_ten_bigint_five_spend_defaults(
    AutorouterAndGlobalCohort::GlobalTenBigintFiveSpendDefaults
)]
#[case::global_unique_tuple_null_semantics(
    AutorouterAndGlobalCohort::GlobalUniqueTupleNullSemantics
)]
#[case::issuer_empty_backfill_and_null_rejection(
    AutorouterAndGlobalCohort::IssuerEmptyBackfillAndNullRejection
)]
#[case::issuer_distinct_claim_pairs_inactive_still_unique(
    AutorouterAndGlobalCohort::IssuerDistinctClaimPairsInactiveStillUnique
)]
#[case::baseline_required_initial_equivalence(
    AutorouterAndGlobalCohort::BaselineRequiredInitialEquivalence
)]
#[case::baseline_zero_revisions_false_retired(
    AutorouterAndGlobalCohort::BaselineZeroRevisionsFalseRetired
)]
#[case::dirty_index_excludes_retired_and_published(
    AutorouterAndGlobalCohort::DirtyIndexExcludesRetiredAndPublished
)]
#[case::observation_text_data_false_conflict_and_order_indexes(
    AutorouterAndGlobalCohort::ObservationTextDataFalseConflictAndOrderIndexes
)]
#[tokio::test]
async fn postgres_autorouter_and_global(#[case] cohort: AutorouterAndGlobalCohort) {
    Box::pin(harness(move |[client]| async move {
        apply_migrations_through(&client, autorouter_and_global_checkpoint_for(cohort)).await;
        run_autorouter_and_global_check(&client, cohort).await;
    }))
    .await;
}
