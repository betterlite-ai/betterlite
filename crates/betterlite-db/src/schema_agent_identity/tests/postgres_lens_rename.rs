async fn insert_rename_fixture_rows(client: &Client) {
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_Engine" (id, version, data) VALUES ($1, 2, '{"engine":"preserved","nested":[1,2]}')"#,
        &[&"engine-b"],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_EngineWorker" (id, token_hash, data) VALUES ($1, $2, '{"worker":"payload","active":true}')"#,
        &[&"worker-b", &"hash-b"],
    )
    .await;
    exec_params(
        client,
        r#"INSERT INTO "LiteLLM_EngineRun" (id, engine_id, created_at, data) VALUES ($1, $2, now(), '{"r":1}')"#,
        &[&"run-b", &"engine-b"],
    )
    .await;
}
async fn capture_pre_rename_oids(client: &Client) -> (String, String, String, String) {
    (
        relation_oid(client, "LiteLLM_Engine").await,
        relation_oid(client, "LiteLLM_EngineRun").await,
        relation_oid(client, "LiteLLM_EngineWorker").await,
        relation_oid(client, "LiteLLM_EngineWorker_token_hash_key").await,
    )
}
async fn assert_rename_preserved_oids(client: &Client, before: (String, String, String, String)) {
    assert_eq!(before.0, relation_oid(client, "LiteLLM_Lens").await);
    assert_eq!(before.1, relation_oid(client, "LiteLLM_LensRun").await);
    assert_eq!(before.2, relation_oid(client, "LiteLLM_LensWorker").await);
    assert_eq!(
        before.3,
        relation_oid(client, "LiteLLM_LensWorker_token_hash_key").await
    );
}
async fn assert_engine_row_preserved(client: &Client) {
    let version: i32 = query_one_row(
        client,
        r#"SELECT version FROM "LiteLLM_Lens" WHERE id = $1"#,
        &[&"engine-b"],
    )
    .await
    .get(0);
    assert_eq!(version, 2_i32);
    let engine_data = scalar_text(
        client,
        r#"SELECT data::text FROM "LiteLLM_Lens" WHERE id = $1"#,
        &[&"engine-b"],
    )
    .await;
    assert_eq!(engine_data, r#"{"engine": "preserved", "nested": [1, 2]}"#);
}

async fn assert_lens_run_preserved(client: &Client) {
    let row = query_one_row(
        client,
        r#"SELECT lens_id, data::text FROM "LiteLLM_LensRun" WHERE id = $1"#,
        &[&"run-b"],
    )
    .await;
    let lens_id: String = row.get(0);
    let data: String = row.get(1);
    assert_eq!(lens_id, "engine-b");
    assert_eq!(data, r#"{"r": 1}"#);
}

async fn assert_worker_row_preserved(client: &Client) {
    let token_hash = scalar_text(
        client,
        r#"SELECT token_hash FROM "LiteLLM_LensWorker" WHERE id = $1"#,
        &[&"worker-b"],
    )
    .await;
    assert_eq!(token_hash, "hash-b");
    let worker_data = scalar_text(
        client,
        r#"SELECT data::text FROM "LiteLLM_LensWorker" WHERE id = $1"#,
        &[&"worker-b"],
    )
    .await;
    assert_eq!(worker_data, r#"{"active": true, "worker": "payload"}"#);
}

async fn assert_rename_preserved_rows(client: &Client) {
    assert_engine_row_preserved(client).await;
    assert_lens_run_preserved(client).await;
    assert_worker_row_preserved(client).await;
}
async fn check_rename_preserves_oids_rows_and_hash_bytes(client: &Client) {
    insert_rename_fixture_rows(client).await;
    let before = capture_pre_rename_oids(client).await;
    batch_execute(client, migration_sql_of(RENAME_LENS.directory)).await;
    assert_rename_preserved_oids(client, before).await;
    assert_rename_preserved_rows(client).await;
}
async fn check_rename_already_final_noop(client: &Client) {
    batch_execute(client, migration_sql_of(RENAME_LENS.directory)).await;
    let before = catalog_snapshot(client).await;
    batch_execute(client, migration_sql_of(RENAME_LENS.directory)).await;
    assert_eq!(before, catalog_snapshot(client).await);
}
async fn check_rename_existing_destination_error(client: &Client) {
    batch_execute(client, r#"CREATE TABLE "LiteLLM_Lens" (id INT);"#).await;
    batch_execute_err(client, migration_sql_of(RENAME_LENS.directory)).await;
    assert!(relation_exists(client, "LiteLLM_Engine").await);
    assert!(!relation_exists(client, "LiteLLM_LensRun").await);
}
async fn check_late_index_collision_rolls_back_do(client: &Client) {
    batch_execute(
        client,
        r#"CREATE TABLE "scratch_collision" (token_hash TEXT);
           CREATE UNIQUE INDEX "LiteLLM_LensWorker_token_hash_key" ON "scratch_collision" (token_hash);"#,
    )
    .await;
    batch_execute_err(client, migration_sql_of(RENAME_LENS.directory)).await;
    assert!(relation_exists(client, "LiteLLM_Engine").await);
    assert!(relation_exists(client, "LiteLLM_EngineRun").await);
    assert!(relation_exists(client, "LiteLLM_EngineWorker").await);
    assert!(!relation_exists(client, "LiteLLM_Lens").await);
}
