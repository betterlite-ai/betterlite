const ADD_MCP_SERVER_INSTRUCTIONS_PREV: &str = "20260414140000_add_mcp_server_instructions";
const HEALTH_CHECK_LATEST_PER_MODEL_INDEX: &str =
    "20260415120000_health_check_latest_per_model_index";
const HEALTH_INDEX_NAME: &str = "LiteLLM_HealthCheckTable_model_id_model_name_checked_at_idx";

async fn insert_health_check_row(
    client: &Client,
    id: &str,
    model_id: &str,
    checked_at_offset_minutes: i64,
) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_HealthCheckTable"
               (health_check_id, model_name, model_id, status, checked_at, updated_at)
           VALUES ($1, $2, $3, 'healthy', now() + ($4::bigint * interval '1 minute'), now())"#,
        &[&id, &"gpt-health", &model_id, &checked_at_offset_minutes],
    )
    .await;
}

#[derive(Clone, Copy)]
enum HealthIndexCohort {
    CheckedAtDescendingAndValid,
    ExplicitTransactionError25001,
}

fn health_index_checkpoint_for(cohort: HealthIndexCohort) -> &'static str {
    match cohort {
        HealthIndexCohort::CheckedAtDescendingAndValid => HEALTH_CHECK_LATEST_PER_MODEL_INDEX,
        HealthIndexCohort::ExplicitTransactionError25001 => ADD_MCP_SERVER_INSTRUCTIONS_PREV,
    }
}

async fn run_health_index_check(client: &Client, cohort: HealthIndexCohort) {
    match cohort {
        HealthIndexCohort::CheckedAtDescendingAndValid => {
            check_checked_at_descending_and_valid(client).await;
        }
        HealthIndexCohort::ExplicitTransactionError25001 => {
            check_explicit_transaction_error_25001(client).await;
        }
    }
}

#[rstest]
#[case::checked_at_descending_and_valid(HealthIndexCohort::CheckedAtDescendingAndValid)]
#[case::explicit_transaction_error_25001(HealthIndexCohort::ExplicitTransactionError25001)]
#[tokio::test]
async fn postgres_concurrent_health_index(#[case] cohort: HealthIndexCohort) {
    harness(move |[client]| async move {
        apply_migrations_through(&client, health_index_checkpoint_for(cohort)).await;
        run_health_index_check(&client, cohort).await;
    })
    .await;
}

async fn check_checked_at_descending_and_valid(client: &Client) {
    let facts = index_facts(client, HEALTH_INDEX_NAME).await;
    assert_eq!(
        facts.columns,
        vec![
            "model_id".to_owned(),
            "model_name".to_owned(),
            "checked_at".to_owned(),
        ]
    );
    assert!(!facts.unique);

    let valid = scalar_bool(
        client,
        "SELECT indisvalid FROM pg_index idx \
         JOIN pg_class ic ON ic.oid = idx.indexrelid WHERE ic.relname = $1",
        &[&HEALTH_INDEX_NAME],
    )
    .await;
    assert!(
        valid,
        "a CONCURRENTLY build that completes leaves the index valid"
    );

    insert_health_check_row(client, "health-older", "model-latest", -10).await;
    insert_health_check_row(client, "health-newer", "model-latest", 0).await;

    let latest = scalar_text(
        client,
        r#"SELECT health_check_id FROM "LiteLLM_HealthCheckTable"
           WHERE model_id = $1
           ORDER BY model_id, model_name, checked_at DESC
           LIMIT 1"#,
        &[&"model-latest"],
    )
    .await;
    assert_eq!(latest, "health-newer");
}

async fn check_explicit_transaction_error_25001(client: &Client) {
    let wrapped = format!(
        "BEGIN;\n{}\nCOMMIT;",
        migration_sql_of(HEALTH_CHECK_LATEST_PER_MODEL_INDEX)
    );
    let error = batch_execute_err(client, &wrapped).await;
    assert_eq!(sql_state(&error), SqlState::ACTIVE_SQL_TRANSACTION.code());
}
