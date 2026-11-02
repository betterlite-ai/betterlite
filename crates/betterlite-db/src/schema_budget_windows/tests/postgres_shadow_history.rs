#[derive(Clone, Copy)]
enum ShadowHistoryCohort {
    RequiredJobInputsNoRangeChecks,
    AttemptJudgeCostZeroAndNullableDetails,
    OneUnstoppedJobIncludingExpired,
    DirectionForwardAllowsOtherDirection,
    GroupIdNullFillBeforeNotNull,
    ExistingGroupIdPreserved,
    StoppedFutureOnlyUnknownBackfill,
    StoppedEqualNowOrExpiredUnchanged,
    BudgetAndComparisonCostDefaults,
    FunnelFourZeroCounters,
    TargetTypeKeyAndRenamedId,
    PartialUniquenessPerTargetTypeAndDirection,
    RouterNamesModelsEmptyAttemptRouterNull,
}

fn shadow_history_checkpoint_for(cohort: ShadowHistoryCohort) -> &'static str {
    match cohort {
        ShadowHistoryCohort::RequiredJobInputsNoRangeChecks
        | ShadowHistoryCohort::AttemptJudgeCostZeroAndNullableDetails
        | ShadowHistoryCohort::OneUnstoppedJobIncludingExpired => ADD_SHADOW_EVAL.directory,
        ShadowHistoryCohort::DirectionForwardAllowsOtherDirection => {
            ADD_SHADOW_EVAL_DIRECTION.directory
        }
        ShadowHistoryCohort::GroupIdNullFillBeforeNotNull => ADD_PROXY_WORKER_HEARTBEAT.directory,
        ShadowHistoryCohort::ExistingGroupIdPreserved
        | ShadowHistoryCohort::StoppedFutureOnlyUnknownBackfill
        | ShadowHistoryCohort::StoppedEqualNowOrExpiredUnchanged => SHADOW_EVAL_MULTI_KEY.directory,
        ShadowHistoryCohort::BudgetAndComparisonCostDefaults
        | ShadowHistoryCohort::FunnelFourZeroCounters => SHADOW_EVAL_COST_COMPARISON.directory,
        ShadowHistoryCohort::TargetTypeKeyAndRenamedId
        | ShadowHistoryCohort::PartialUniquenessPerTargetTypeAndDirection => {
            SHADOW_EVAL_TYPED_TARGETS.directory
        }
        ShadowHistoryCohort::RouterNamesModelsEmptyAttemptRouterNull => {
            SHADOW_EVAL_MODEL_SCOPE.directory
        }
    }
}

async fn insert_shadow_job(client: &Client, id: &str, api_key_id: &str, offset: &str) {
    exec_params(
        client,
        &format!(
            r#"INSERT INTO "LiteLLM_ShadowEvalJob" (id, api_key_id, router_name, judge_model, shadow_percentage, max_turns, ends_at) VALUES ($1, $2, $3, $4, $5, $6, now() {offset})"#
        ),
        &[&id, &api_key_id, &"router-a", &"judge-a", &10.0_f64, &5_i32],
    )
    .await;
}

async fn check_required_job_inputs_no_range_checks(client: &Client) {
    assert_exec_params_err_state(
        client,
        r#"INSERT INTO "LiteLLM_ShadowEvalJob" (id, api_key_id, judge_model, shadow_percentage, max_turns, ends_at) VALUES ($1, $2, $3, $4, $5, now())"#,
        &[&"job-missing-router", &"key-a", &"judge-a", &10.0_f64, &5_i32],
        SqlState::NOT_NULL_VIOLATION,
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_ShadowEvalJob" (id, api_key_id, router_name, judge_model, shadow_percentage, max_turns, ends_at) VALUES ($1, $2, $3, $4, $5, $6, now())"#,
        &[&"job-extreme", &"key-a", &"router-a", &"judge-a", &150.5_f64, &(-5_i32)],
    )
    .await;
    let row = query_one_row(
        client,
        r#"SELECT shadow_percentage::text, max_turns FROM "LiteLLM_ShadowEvalJob" WHERE id = $1"#,
        &[&"job-extreme"],
    )
    .await;
    let shadow_percentage: String = row.get(0);
    let max_turns: i32 = row.get(1);
    assert_eq!(shadow_percentage, "150.5");
    assert_eq!(max_turns, -5_i32);
}

async fn check_attempt_judge_cost_zero_and_nullable_details(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_ShadowEvalAttempt" (id, job_id, request_id, outcome) VALUES ($1, $2, $3, $4)"#,
        &[&"attempt-a", &"job-a", &"req-a", &"shadow_better"],
    )
    .await;
    assert_column(
        client,
        "LiteLLM_ShadowEvalAttempt",
        float8_default("judge_cost", "0"),
    )
    .await;
    let row = query_one_row(
        client,
        r#"SELECT tier, real_model, shadow_model, confidence, error, judge_cost::text FROM "LiteLLM_ShadowEvalAttempt" WHERE id = $1"#,
        &[&"attempt-a"],
    )
    .await;
    let tier: Option<String> = row.get(0);
    let real_model: Option<String> = row.get(1);
    let shadow_model: Option<String> = row.get(2);
    let confidence: Option<f64> = row.get(3);
    let error: Option<String> = row.get(4);
    let judge_cost: String = row.get(5);
    assert_eq!(tier, None);
    assert_eq!(real_model, None);
    assert_eq!(shadow_model, None);
    assert_eq!(confidence, None);
    assert_eq!(error, None);
    assert_eq!(judge_cost, "0");
}

async fn check_one_unstopped_job_including_expired(client: &Client) {
    insert_shadow_job(client, "job-active", "key-shared", "+ interval '1 hour'").await;
    let conflict = exec_params_err(
        client,
        r#"INSERT INTO "LiteLLM_ShadowEvalJob" (id, api_key_id, router_name, judge_model, shadow_percentage, max_turns, ends_at) VALUES ($1, $2, $3, $4, $5, $6, now() - interval '1 hour')"#,
        &[&"job-expired", &"key-shared", &"router-a", &"judge-a", &10.0_f64, &5_i32],
    )
    .await;
    assert_eq!(sql_state(&conflict), SqlState::UNIQUE_VIOLATION.code());
}

async fn check_direction_forward_allows_other_direction(client: &Client) {
    insert_shadow_job(client, "job-forward", "key-dir", "+ interval '1 hour'").await;
    let direction = scalar_text(
        client,
        r#"SELECT direction FROM "LiteLLM_ShadowEvalJob" WHERE id = $1"#,
        &[&"job-forward"],
    )
    .await;
    assert_eq!(direction, "forward");
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_ShadowEvalJob" (id, api_key_id, router_name, judge_model, shadow_percentage, max_turns, direction, ends_at) VALUES ($1, $2, $3, $4, $5, $6, $7, now() + interval '1 hour')"#,
        &[
            &"job-backward",
            &"key-dir",
            &"router-a",
            &"judge-a",
            &10.0_f64,
            &5_i32,
            &"backward",
        ],
    )
    .await;
    let conflict = exec_params_err(
        client,
        r#"INSERT INTO "LiteLLM_ShadowEvalJob" (id, api_key_id, router_name, judge_model, shadow_percentage, max_turns, ends_at) VALUES ($1, $2, $3, $4, $5, $6, now() + interval '1 hour')"#,
        &[&"job-forward-2", &"key-dir", &"router-a", &"judge-a", &10.0_f64, &5_i32],
    )
    .await;
    assert_eq!(sql_state(&conflict), SqlState::UNIQUE_VIOLATION.code());
}

async fn check_shadow_eval_scalar_backfill(
    client: &Client,
    id: &str,
    migration_directory: &str,
    select_column: &str,
    expected: Option<&str>,
) {
    batch_execute(client, migration_sql_of(migration_directory)).await;
    let actual = scalar_opt_text(
        client,
        &format!(r#"SELECT {select_column} FROM "LiteLLM_ShadowEvalJob" WHERE id = $1"#),
        &[&id],
    )
    .await;
    assert_eq!(actual.as_deref(), expected);
}

async fn check_budget_and_comparison_cost_defaults(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_ShadowEvalJob" (id, api_key_id, router_name, judge_model, shadow_percentage, max_turns, group_id, ends_at) VALUES ($1, $2, $3, $4, $5, $6, $1, now() + interval '1 hour')"#,
        &[&"job-budget", &"key-budget", &"router-a", &"judge-a", &10.0_f64, &5_i32],
    )
    .await;
    assert_column(
        client,
        "LiteLLM_ShadowEvalJob",
        ("max_budget", "double precision", true, None, "float8", None),
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_ShadowEvalAttempt" (id, job_id, request_id, outcome) VALUES ($1, $2, $3, $4)"#,
        &[&"attempt-cost", &"job-budget", &"req-cost", &"shadow_better"],
    )
    .await;
    assert_columns(
        client,
        "LiteLLM_ShadowEvalAttempt",
        &[
            float8_default("shadow_cost", "0"),
            float8_default("real_classifier_cost", "0"),
            float8_default("shadow_classifier_cost", "0"),
            bool_false("real_cache_hit"),
            ("real_cost", "double precision", true, None, "float8", None),
        ],
    )
    .await;
}

async fn check_funnel_four_zero_counters(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_ShadowEvalFunnel" (job_id) VALUES ($1)"#,
        &[&"job-funnel"],
    )
    .await;
    assert_columns(
        client,
        "LiteLLM_ShadowEvalFunnel",
        &[
            int4_zero("not_sampled"),
            int4_zero("unjudgeable"),
            int4_zero("shed"),
            int4_zero("withheld"),
        ],
    )
    .await;
}

async fn check_target_type_key_and_renamed_id(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_ShadowEvalJob" (id, target_id, router_name, judge_model, shadow_percentage, max_turns, group_id, ends_at) VALUES ($1, $2, $3, $4, $5, $6, $1, now() + interval '1 hour')"#,
        &[&"job-target", &"key-target", &"router-a", &"judge-a", &10.0_f64, &5_i32],
    )
    .await;
    let target_type = scalar_text(
        client,
        r#"SELECT target_type FROM "LiteLLM_ShadowEvalJob" WHERE id = $1"#,
        &[&"job-target"],
    )
    .await;
    assert_eq!(target_type, "key");
    let legacy_columns = query_one_row(
        client,
        "SELECT count(*) FROM information_schema.columns \
         WHERE table_name = 'LiteLLM_ShadowEvalJob' AND column_name = 'api_key_id'",
        &[],
    )
    .await;
    let legacy_column_count: i64 = legacy_columns.get(0);
    assert_eq!(legacy_column_count, 0);
}

async fn check_partial_uniqueness_per_target_type_and_direction(client: &Client) {
    assert_repeat_insert_unique_violation(
        client,
        r#"INSERT INTO "LiteLLM_ShadowEvalJob" (id, target_id, router_name, judge_model, shadow_percentage, max_turns, group_id, ends_at) VALUES ($1, $2, $3, $4, $5, $6, $1, now() + interval '1 hour')"#,
        &[&"job-ta", &"target-x", &"router-a", &"judge-a", &10.0_f64, &5_i32],
        &[&"job-tb", &"target-x", &"router-a", &"judge-a", &10.0_f64, &5_i32],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_ShadowEvalJob" (id, target_type, target_id, router_name, judge_model, shadow_percentage, max_turns, group_id, ends_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $1, now() + interval '1 hour')"#,
        &[
            &"job-tc",
            &"team",
            &"target-x",
            &"router-a",
            &"judge-a",
            &10.0_f64,
            &5_i32,
        ],
    )
    .await;
}

async fn check_router_names_models_empty_attempt_router_null(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_ShadowEvalJob" (id, target_id, router_name, judge_model, shadow_percentage, max_turns, group_id, ends_at) VALUES ($1, $2, $3, $4, $5, $6, $1, now() + interval '1 hour')"#,
        &[&"job-scope", &"key-scope", &"router-a", &"judge-a", &10.0_f64, &5_i32],
    )
    .await;
    assert_columns(
        client,
        "LiteLLM_ShadowEvalJob",
        &[empty_text_array("router_names"), empty_text_array("models")],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_ShadowEvalAttempt" (id, job_id, request_id, outcome) VALUES ($1, $2, $3, $4)"#,
        &[&"attempt-scope", &"job-scope", &"req-scope", &"shadow_better"],
    )
    .await;
    assert_column(
        client,
        "LiteLLM_ShadowEvalAttempt",
        nullable_text("router_name"),
    )
    .await;
    let router_name = scalar_opt_text(
        client,
        r#"SELECT router_name FROM "LiteLLM_ShadowEvalAttempt" WHERE id = $1"#,
        &[&"attempt-scope"],
    )
    .await;
    assert_eq!(router_name, None);
}

async fn check_group_id_null_fill_before_not_null(client: &Client) {
    insert_shadow_job(client, "job-g1", "key-g1", "+ interval '1 hour'").await;
    check_shadow_eval_scalar_backfill(
        client,
        "job-g1",
        SHADOW_EVAL_MULTI_KEY.directory,
        "group_id",
        Some("job-g1"),
    )
    .await;
    assert_column(
        client,
        "LiteLLM_ShadowEvalJob",
        ("group_id", "text", false, None, "text", None),
    )
    .await;
}

async fn check_existing_group_id_preserved(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_ShadowEvalJob" (id, api_key_id, router_name, judge_model, shadow_percentage, max_turns, group_id, ends_at) VALUES ($1, $2, $3, $4, $5, $6, $7, now() + interval '1 hour')"#,
        &[
            &"job-g2",
            &"key-g2",
            &"router-a",
            &"judge-a",
            &10.0_f64,
            &5_i32,
            &"custom-group",
        ],
    )
    .await;
    check_shadow_eval_scalar_backfill(
        client,
        "job-g2",
        SHADOW_EVAL_MULTI_KEY.directory,
        "group_id",
        Some("custom-group"),
    )
    .await;
}

const STOPPED_AT_ALIGNMENT_RETRIES: u32 = 20_000;

async fn align_transaction_to_millisecond_boundary(client: &Client) {
    batch_execute(client, "BEGIN; SET LOCAL TIME ZONE 'UTC';").await;
    for attempts_remaining in (1..=STOPPED_AT_ALIGNMENT_RETRIES).rev() {
        let sub_millisecond_remainder: i64 = client
            .query_one(
                "SELECT extract(microseconds from now())::bigint % 1000",
                &[],
            )
            .await
            .unwrap()
            .get(0);
        if sub_millisecond_remainder == 0 {
            return;
        }
        assert!(
            attempts_remaining > 1,
            "transaction now() never landed on a millisecond boundary after {STOPPED_AT_ALIGNMENT_RETRIES} tries"
        );
        batch_execute(client, "ROLLBACK; BEGIN; SET LOCAL TIME ZONE 'UTC';").await;
    }
}

async fn seed_stopped_at_offset_rows(client: &Client) {
    align_transaction_to_millisecond_boundary(client).await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_ShadowEvalJob" (id, api_key_id, router_name, judge_model, shadow_percentage, max_turns, group_id, stopped_at, ends_at) VALUES ($1, $2, $3, $4, $5, $6, $1, now(), now() + interval '1 hour')"#,
        &[&"job-stop-future", &"key-stop", &"router-a", &"judge-a", &10.0_f64, &5_i32],
    )
    .await;
    batch_execute(
        client,
        r#"INSERT INTO "LiteLLM_ShadowEvalJob" (id, api_key_id, router_name, judge_model, shadow_percentage, max_turns, group_id, stopped_at, ends_at) VALUES ('job-stop-equal', 'key-stop', 'router-a', 'judge-a', 10.0, 5, 'job-stop-equal', now(), now() AT TIME ZONE 'utc')"#,
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_ShadowEvalJob" (id, api_key_id, router_name, judge_model, shadow_percentage, max_turns, group_id, stopped_at, ends_at) VALUES ($1, $2, $3, $4, $5, $6, $1, now(), now() - interval '1 hour')"#,
        &[&"job-stop-expired", &"key-stop", &"router-a", &"judge-a", &10.0_f64, &5_i32],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_ShadowEvalJob" (id, api_key_id, router_name, judge_model, shadow_percentage, max_turns, group_id, ends_at) VALUES ($1, $2, $3, $4, $5, $6, $1, now() + interval '1 hour')"#,
        &[
            &"job-stop-unstopped-future",
            &"key-stop",
            &"router-a",
            &"judge-a",
            &10.0_f64,
            &5_i32,
        ],
    )
    .await;
}

async fn check_stopped_at_offset_backfill(client: &Client) {
    seed_stopped_at_offset_rows(client).await;
    batch_execute(
        client,
        migration_sql_of(ADD_SHADOW_EVAL_STOPPED_BY.directory),
    )
    .await;
    let future = scalar_opt_text(
        client,
        r#"SELECT stopped_by FROM "LiteLLM_ShadowEvalJob" WHERE id = $1"#,
        &[&"job-stop-future"],
    )
    .await;
    let equal = scalar_opt_text(
        client,
        r#"SELECT stopped_by FROM "LiteLLM_ShadowEvalJob" WHERE id = $1"#,
        &[&"job-stop-equal"],
    )
    .await;
    let expired = scalar_opt_text(
        client,
        r#"SELECT stopped_by FROM "LiteLLM_ShadowEvalJob" WHERE id = $1"#,
        &[&"job-stop-expired"],
    )
    .await;
    let unstopped_future = scalar_opt_text(
        client,
        r#"SELECT stopped_by FROM "LiteLLM_ShadowEvalJob" WHERE id = $1"#,
        &[&"job-stop-unstopped-future"],
    )
    .await;
    batch_execute(client, "COMMIT;").await;
    assert_eq!(future.as_deref(), Some("unknown"));
    assert_eq!(equal, None);
    assert_eq!(expired, None);
    assert_eq!(unstopped_future, None);
}

async fn run_shadow_history_check(client: &Client, cohort: ShadowHistoryCohort) {
    use ShadowHistoryCohort::*;
    let check: Pin<Box<dyn Future<Output = ()> + Send + '_>> = match cohort {
        RequiredJobInputsNoRangeChecks => {
            Box::pin(check_required_job_inputs_no_range_checks(client))
        }
        AttemptJudgeCostZeroAndNullableDetails => {
            Box::pin(check_attempt_judge_cost_zero_and_nullable_details(client))
        }
        OneUnstoppedJobIncludingExpired => {
            Box::pin(check_one_unstopped_job_including_expired(client))
        }
        DirectionForwardAllowsOtherDirection => {
            Box::pin(check_direction_forward_allows_other_direction(client))
        }
        GroupIdNullFillBeforeNotNull => Box::pin(check_group_id_null_fill_before_not_null(client)),
        ExistingGroupIdPreserved => Box::pin(check_existing_group_id_preserved(client)),
        StoppedFutureOnlyUnknownBackfill | StoppedEqualNowOrExpiredUnchanged => {
            Box::pin(check_stopped_at_offset_backfill(client))
        }
        BudgetAndComparisonCostDefaults => {
            Box::pin(check_budget_and_comparison_cost_defaults(client))
        }
        FunnelFourZeroCounters => Box::pin(check_funnel_four_zero_counters(client)),
        TargetTypeKeyAndRenamedId => Box::pin(check_target_type_key_and_renamed_id(client)),
        PartialUniquenessPerTargetTypeAndDirection => Box::pin(
            check_partial_uniqueness_per_target_type_and_direction(client),
        ),
        RouterNamesModelsEmptyAttemptRouterNull => {
            Box::pin(check_router_names_models_empty_attempt_router_null(client))
        }
    };
    check.await;
}

#[rstest]
#[case::required_job_inputs_no_range_checks(ShadowHistoryCohort::RequiredJobInputsNoRangeChecks)]
#[case::attempt_judge_cost_zero_and_nullable_details(
    ShadowHistoryCohort::AttemptJudgeCostZeroAndNullableDetails
)]
#[case::one_unstopped_job_including_expired(ShadowHistoryCohort::OneUnstoppedJobIncludingExpired)]
#[case::direction_forward_allows_other_direction(
    ShadowHistoryCohort::DirectionForwardAllowsOtherDirection
)]
#[case::group_id_null_fill_before_not_null(ShadowHistoryCohort::GroupIdNullFillBeforeNotNull)]
#[case::existing_group_id_preserved(ShadowHistoryCohort::ExistingGroupIdPreserved)]
#[case::stopped_future_only_unknown_backfill(ShadowHistoryCohort::StoppedFutureOnlyUnknownBackfill)]
#[case::stopped_equal_now_or_expired_unchanged(
    ShadowHistoryCohort::StoppedEqualNowOrExpiredUnchanged
)]
#[case::budget_and_comparison_cost_defaults(ShadowHistoryCohort::BudgetAndComparisonCostDefaults)]
#[case::funnel_four_zero_counters(ShadowHistoryCohort::FunnelFourZeroCounters)]
#[case::target_type_key_and_renamed_id(ShadowHistoryCohort::TargetTypeKeyAndRenamedId)]
#[case::partial_uniqueness_per_target_type_and_direction(
    ShadowHistoryCohort::PartialUniquenessPerTargetTypeAndDirection
)]
#[case::router_names_models_empty_attempt_router_null(
    ShadowHistoryCohort::RouterNamesModelsEmptyAttemptRouterNull
)]
#[tokio::test]
async fn postgres_shadow_history(#[case] cohort: ShadowHistoryCohort) {
    Box::pin(harness(move |[client]| async move {
        apply_migrations_through(&client, shadow_history_checkpoint_for(cohort)).await;
        run_shadow_history_check(&client, cohort).await;
    }))
    .await;
}
