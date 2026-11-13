use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::batches::records::BatchObject;
use betterlite_openai::files::records::FileObject;
use betterlite_openai::fine_tuning::job::FineTuneJob;
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use clap::Parser;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::JoinSet;
use uuid::Uuid;

use crate::budget_permission_rows::JsonData;
use crate::stored_cipher::DbCipher;

use super::super::*;

pub(crate) type FullRow =
    ManagedResourceRow<FileObject, BatchObject, FineTuneJob, ResponsesApiResponse>;
pub(crate) type ConnectionTasks = JoinSet<Result<(), tokio_postgres::Error>>;

pub(crate) const CIPHERTEXT: &str = "YRxZqp_n51DmLFJrE5ewMwpkUQt7RBVfz86OO7dO1hEoqpx1r1L8ZbJr__NBLz_I7IRDN-1hlIRQTi6LoL0y3bLHgP8qyPKS35jUEso=";
pub(crate) const PLAINTEXT: &str = "Bearer row-secret";
pub(crate) const INJECTED_REQUEST_ID: u128 = 42;
pub(crate) const BATCH_OBJECT: &str = r#"{"id":"b","completion_window":"24h","created_at":1,"endpoint":"/v1/chat/completions","input_file_id":"f","object":"batch","status":"validating"}"#;
pub(crate) const FINE_TUNING_OBJECT: &str = r#"{"id":"ft","created_at":1,"hyperparameters":{"n_epochs":3},"model":"m","object":"fine_tuning.job","organization_id":"o","result_files":[],"status":"queued","training_file":"f"}"#;
pub(crate) const RESPONSE_OBJECT: &str =
    r#"{"id":"r","created_at":1,"object":"response","output":[]}"#;

pub(crate) fn data(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap()
}

pub(crate) fn field_of(row: &JsonData, name: &str) -> JsonData {
    match row {
        JsonData::Object(fields) => fields.get(name).cloned().unwrap_or(JsonData::Null),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Array(_) => JsonData::Null,
    }
}

pub(crate) fn set_field(row: &mut JsonData, name: &str, value: JsonData) {
    if let JsonData::Object(fields) = row {
        fields.insert(name.to_owned(), value);
    }
}

pub(crate) fn remove_field(row: &mut JsonData, name: &str) {
    if let JsonData::Object(fields) = row {
        fields.remove(name);
    }
}

pub(crate) fn cipher_with(key: &str) -> DbCipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(key.to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

pub(crate) fn decode_with(
    stored: &str,
    cipher: &DbCipher,
) -> Result<FullRow, ManagedRowsDecodeError> {
    let dependencies = ManagedRowDependencies {
        cipher,
        error_request_id: Uuid::from_u128(INJECTED_REQUEST_ID),
    };
    decode_managed_resource_rows::<FileObject, BatchObject, FineTuneJob, ResponsesApiResponse>(
        stored.as_bytes(),
        &dependencies,
    )
}

pub(crate) fn decode(stored: &str) -> Result<FullRow, ManagedRowsDecodeError> {
    let cipher = cipher_with("managed-row-key");
    decode_with(stored, &cipher)
}

pub(crate) fn envelope(kind: &str, row: &JsonData) -> String {
    format!(
        "{{\"kind\":\"{kind}\",\"row\":{}}}",
        serde_json::to_string(row).unwrap()
    )
}

pub(crate) fn decode_envelope(
    kind: &str,
    row: &JsonData,
) -> Result<FullRow, ManagedRowsDecodeError> {
    decode(&envelope(kind, row))
}

pub(crate) fn accepts(kind: &str, row: &JsonData) -> bool {
    decode_envelope(kind, row).is_ok()
}

fn dumped_row(decoded: &FullRow) -> JsonData {
    let text = serde_json::to_string(decoded).unwrap();
    field_of(&data(&text), "row")
}

pub(crate) fn dumped(kind: &str, row: &JsonData) -> JsonData {
    dumped_row(&decode_envelope(kind, row).unwrap())
}

pub(crate) fn assert_field_cases(kind: &str, base: JsonData, spec: (&str, &str, &str, &str, bool)) {
    let (field, default, supplied, expected, null_ok) = spec;
    let mut row = base;
    assert_eq!(field_of(&dumped(kind, &row), field), data(default));
    set_field(&mut row, field, data(supplied));
    assert_eq!(field_of(&dumped(kind, &row), field), data(expected));
    set_field(&mut row, field, JsonData::Null);
    assert_eq!(accepts(kind, &row), null_ok, "null {field}");
}

pub(crate) fn assert_required_field(kind: &str, base: JsonData, field: &str) {
    let mut row = base;
    remove_field(&mut row, field);
    assert!(!accepts(kind, &row), "missing {field}");
    set_field(&mut row, field, JsonData::Null);
    assert!(!accepts(kind, &row), "null {field}");
}

pub(crate) fn assert_required_nullable(kind: &str, base: JsonData, field: &str) {
    let mut row = base;
    remove_field(&mut row, field);
    assert!(!accepts(kind, &row), "missing {field}");
    set_field(&mut row, field, JsonData::Null);
    assert!(accepts(kind, &row), "null {field}");
}

pub(crate) fn assert_secret_map_redacted() {
    let wrong = cipher_with("other-key");
    let row = data(&format!(
        r#"{{"server_id":"s","transport":"sse","static_headers":"{CIPHERTEXT}"}}"#
    ));
    let error = decode_with(&envelope("mcp_server", &row), &wrong).unwrap_err();
    let text = format!("{error} {error:?}");
    assert!(!text.contains(PLAINTEXT));
    assert!(!text.contains(CIPHERTEXT));
    let right = cipher_with("managed-row-key");
    let stored = envelope(
        "mcp_server",
        &data(
            r#"{"server_id":"s","transport":"sse","static_headers":{"Authorization":"Bearer row-secret"}}"#,
        ),
    );
    let decoded = decode_with(&stored, &right).unwrap();
    assert!(!format!("{decoded:?}").contains(PLAINTEXT));
}

pub(crate) fn assert_byte_and_depth_limits() {
    let cipher = cipher_with("managed-row-key");
    let tag = r#"{"kind":"tag","row":{"tag_name":"t"}}"#;
    let padding = " ".repeat(0x0010_0000_usize.saturating_sub(tag.len()));
    let at_limit = format!("{tag}{padding}");
    decode_with(&at_limit, &cipher).unwrap();
    let oversized = format!("{at_limit} ");
    assert!(matches!(
        decode_with(&oversized, &cipher),
        Err(ManagedRowsDecodeError::TooLarge { len: 0x0010_0001 })
    ));
    for (levels, fits) in [(62_usize, true), (63, false)] {
        let nested = format!(
            r#"{{"kind":"tag","row":{{"tag_name":"t","model_info":{}0{}}}}}"#,
            "[".repeat(levels),
            "]".repeat(levels)
        );
        let result = decode_with(&nested, &cipher);
        if fits {
            result.unwrap();
        } else {
            assert!(matches!(result, Err(ManagedRowsDecodeError::TooDeep)));
        }
    }
}

pub(crate) fn assert_input_limits() {
    assert_byte_and_depth_limits();
    let cipher = cipher_with("managed-row-key");
    let dependencies = ManagedRowDependencies {
        cipher: &cipher,
        error_request_id: Uuid::from_u128(INJECTED_REQUEST_ID),
    };
    assert!(matches!(
        decode_managed_resource_rows::<FileObject, BatchObject, FineTuneJob, ResponsesApiResponse>(
            &[0xff],
            &dependencies
        ),
        Err(ManagedRowsDecodeError::InvalidUtf8(_))
    ));
    assert!(matches!(
        decode_with(r#"{"kind":"not_a_row","row":{}}"#, &cipher),
        Err(ManagedRowsDecodeError::UnknownKind { .. })
    ));
    let wrapped = format!(
        r#"{{"kind":"mcp_server","row":{{"server_id":"s","transport":"sse","static_headers":"{CIPHERTEXT}","extra":{}0{}}}}}"#,
        "[".repeat(63),
        "]".repeat(63)
    );
    let error = decode_with(&wrapped, &cipher).unwrap_err();
    assert!(matches!(&error, ManagedRowsDecodeError::TooDeep));
    assert!(!format!("{error} {error:?}").contains(PLAINTEXT));
}

pub(crate) fn mcp_server(row: &JsonData) -> McpServerTable {
    let FullRow::McpServer(server) = decode_envelope("mcp_server", row).unwrap() else {
        panic!("expected an mcp server row");
    };
    *server
}

pub(crate) fn auto_router(row: &JsonData) -> AutoRouterSession {
    let FullRow::AutoRouterSession(session) = decode_envelope("autorouter_session", row).unwrap()
    else {
        panic!("expected an autorouter session row");
    };
    *session
}

pub(crate) fn skill_table(row: &JsonData) -> SkillTable {
    let FullRow::Skill(skill) = decode_envelope("skill", row).unwrap() else {
        panic!("expected a skill row");
    };
    *skill
}

pub(crate) fn spend_log(row: &JsonData) -> SpendLog {
    let FullRow::SpendLog(log) = decode_envelope("spend_log", row).unwrap() else {
        panic!("expected a spend log row");
    };
    *log
}

pub(crate) fn error_log(row: &JsonData) -> ErrorLog {
    let FullRow::ErrorLog(log) = decode_envelope("error_log", row).unwrap() else {
        panic!("expected an error log row");
    };
    *log
}

pub(crate) fn tag_table(row: &JsonData) -> TagTable {
    let FullRow::Tag(tag) = decode_envelope("tag", row).unwrap() else {
        panic!("expected a tag row");
    };
    *tag
}

pub(crate) fn managed_file_base() -> JsonData {
    data(r#"{"unified_file_id":"u","model_mappings":{},"flat_model_file_ids":[]}"#)
}

pub(crate) fn managed_object_base() -> JsonData {
    data(&format!(
        r#"{{"unified_object_id":"o","model_object_id":"m","file_purpose":"batch","file_object":{BATCH_OBJECT}}}"#
    ))
}

pub(crate) fn singular_base() -> JsonData {
    data(r#"{"unified_resource_id":"u","model_mappings":{},"flat_model_resource_ids":[]}"#)
}

pub(crate) fn plural_base() -> JsonData {
    data(r#"{"vector_store_id":"v","custom_llm_provider":"openai"}"#)
}

pub(crate) fn mcp_server_base() -> JsonData {
    data(r#"{"server_id":"s","transport":"sse"}"#)
}

pub(crate) fn autorouter_base() -> JsonData {
    data(
        r#"{"api_key":"k","session_id":"s","router_name":"r","router_type":"t","first_turn_at":"2026-01-01T00:00:00Z","last_turn_at":"2026-01-01T00:00:00Z","last_model":"m","turns":3,"spend":1.5,"saved_spend":0.5,"classifier_cost":0.25,"tier_turns":{"a":2},"baseline_models":{}}"#,
    )
}

pub(crate) fn spend_log_base() -> JsonData {
    data(
        r#"{"request_id":"r","api_key":"k","call_type":"completion","startTime":null,"endTime":null,"messages":null,"response":null}"#,
    )
}

pub(crate) fn error_log_base() -> JsonData {
    data(r#"{"startTime":null,"endTime":null}"#)
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn postgres_test_url() -> Option<SecretString> {
    Environment::parse_from(["managed-resource-rows"])
        .url
        .map(SecretString::from)
}

pub(crate) async fn provision() -> (PostgresFixture, tokio_postgres::Client, ConnectionTasks) {
    let fixture = PostgresFixture::provision_schema(postgres_test_url().as_ref())
        .resolve()
        .await
        .unwrap();
    let (client, connection) =
        tokio_postgres::connect(fixture.options.url.expose_secret(), tokio_postgres::NoTls)
            .await
            .unwrap();
    let mut tasks = JoinSet::new();
    tasks.spawn(connection);
    client.batch_execute(fixtures::SCHEMA).await.unwrap();
    client.batch_execute(SEED).await.unwrap();
    (fixture, client, tasks)
}

pub(crate) async fn shutdown(
    fixture: PostgresFixture,
    client: tokio_postgres::Client,
    mut tasks: ConnectionTasks,
) {
    drop(client);
    tasks.join_next().await.unwrap().unwrap().unwrap();
    fixture.close_async().await.unwrap();
}

pub(crate) async fn assert_pinned_rows(client: &tokio_postgres::Client) {
    let file_json = row_json(client, "LiteLLM_ManagedFileTable", "id = 'f1'").await;
    assert_managed_file(&row_value(&file_json, "managed_file"));
    let empty_file_json = row_json(client, "LiteLLM_ManagedFileTable", "id = 'f2'").await;
    assert_managed_file_defaults(&row_value(&empty_file_json, "managed_file"));
    let object_json = row_json(client, "LiteLLM_ManagedObjectTable", "id = 'o1'").await;
    assert_managed_object(&row_value(&object_json, "managed_object"));
    let singular_json = row_json(client, "LiteLLM_ManagedVectorStoreTable", "id = 'v1'").await;
    assert_singular_store(&row_value(&singular_json, "managed_vector_store"));
    let plural_json = row_json(
        client,
        "LiteLLM_ManagedVectorStoresTable",
        "vector_store_id = 'v1'",
    )
    .await;
    assert_plural_store(&row_value(&plural_json, "managed_vector_stores"));
    let server_json = row_json(client, "LiteLLM_MCPServerTable", "server_id = 's1'").await;
    assert_mcp_server(&row_value(&server_json, "mcp_server"));
    let router_json = row_json(client, "LiteLLM_AutoRouterSession", "session_id = 'r1'").await;
    assert_autorouter(&row_value(&router_json, "autorouter_session"));
    let spend_json = row_json(client, "LiteLLM_SpendLogs", "request_id = 'p1'").await;
    assert_spend_log(&row_value(&spend_json, "spend_log"));
    let error_json = row_json(client, "LiteLLM_ErrorLogs", "request_id = 'e1'").await;
    assert_error_log(&row_value(&error_json, "error_log"));
    let tag_json = row_json(client, "LiteLLM_TagTable", "tag_name = 't1'").await;
    assert_tag(&row_value(&tag_json, "tag"));
    assert_skill(client).await;
}

fn assert_managed_file(row: &JsonData) {
    assert_eq!(field_of(row, "flat_model_file_ids"), data(r#"["mf1"]"#));
    assert_eq!(field_of(row, "storage_backend"), data(r#""s3""#));
    assert_eq!(field_of(row, "team_id"), JsonData::Null);
}

fn assert_managed_file_defaults(row: &JsonData) {
    assert_eq!(field_of(row, "file_object"), JsonData::Null);
    assert_eq!(field_of(row, "flat_model_file_ids"), data("[]"));
    assert_eq!(field_of(row, "created_by"), JsonData::Null);
}

fn assert_managed_object(row: &JsonData) {
    assert_eq!(field_of(row, "file_purpose"), data(r#""batch""#));
    assert_ne!(
        field_of(&field_of(row, "file_object"), "completion_window"),
        JsonData::Null
    );
    assert_eq!(field_of(row, "org_id"), JsonData::Null);
}

fn assert_singular_store(row: &JsonData) {
    assert_eq!(field_of(row, "resource_object"), data("5"));
    assert_eq!(field_of(row, "flat_model_resource_ids"), data("[]"));
}

fn assert_plural_store(row: &JsonData) {
    assert_eq!(field_of(row, "litellm_params"), data(r#"{"index":"i"}"#));
    assert_eq!(field_of(row, "is_config"), data("false"));
}

fn assert_mcp_server(row: &JsonData) {
    assert_eq!(field_of(row, "transport"), data(r#""sse""#));
    assert_eq!(field_of(row, "status"), data(r#""unknown""#));
    assert_eq!(field_of(row, "static_headers"), data("{}"));
    assert_eq!(field_of(row, "env"), data("{}"));
    assert_eq!(field_of(row, "approval_status"), data(r#""active""#));
}

fn assert_autorouter(row: &JsonData) {
    assert_eq!(field_of(row, "savings_estimated_turns"), data("0"));
    assert_eq!(
        field_of(row, "savings_estimated_baseline_models"),
        data("{}")
    );
    assert_eq!(field_of(row, "spend"), data("0.5"));
}

fn assert_spend_log(row: &JsonData) {
    assert_eq!(field_of(row, "model"), data(r#""m""#));
    assert_eq!(field_of(row, "cache_hit"), data(r#""""#));
    assert_eq!(field_of(row, "metadata"), data(r#"{"trace":"t"}"#));
}

fn assert_error_log(row: &JsonData) {
    assert_eq!(field_of(row, "status_code"), data(r#""500""#));
    assert_eq!(field_of(row, "request_kwargs"), data(r#"{"error":"boom"}"#));
}

fn assert_tag(row: &JsonData) {
    assert_eq!(field_of(row, "models"), data("[]"));
    assert_eq!(field_of(row, "spend"), data("0.0"));
    assert_eq!(field_of(row, "budget_id"), JsonData::Null);
}

async fn assert_skill(client: &tokio_postgres::Client) {
    let skill_row = client
        .query_one(
            "SELECT row_to_json(row)::TEXT, row.file_content FROM \"LiteLLM_SkillsTable\" row WHERE row.skill_id = 'k1'",
            &[],
        )
        .await
        .unwrap();
    let mut stored_skill = data(skill_row.get(0));
    set_field(
        &mut stored_skill,
        "file_content",
        JsonData::String(
            String::from_utf8(skill_row.get::<usize, Option<Vec<u8>>>(1).unwrap()).unwrap(),
        ),
    );
    let skill = dumped("skill", &stored_skill);
    assert_eq!(field_of(&skill, "file_content"), data(r#""a\u0000b""#));
}

pub(crate) fn row_value(stored: &str, kind: &str) -> JsonData {
    dumped(kind, &data(stored))
}

pub(crate) async fn row_json(client: &tokio_postgres::Client, table: &str, filter: &str) -> String {
    let query = format!("SELECT row_to_json(row)::TEXT FROM \"{table}\" row WHERE {filter}");
    client.query_one(&query, &[]).await.unwrap().get(0)
}

const SEED: &str = r#"
INSERT INTO "LiteLLM_ManagedFileTable" (id, unified_file_id, file_object, model_mappings, flat_model_file_ids, storage_backend, updated_at)
VALUES ('f1', 'u1', '{"id": "f", "bytes": 3, "created_at": 1, "filename": "a.txt", "object": "file", "purpose": "batch"}', '{"m": "mf1"}', ARRAY['mf1'], 's3', '2026-01-01T00:00:00');
INSERT INTO "LiteLLM_ManagedFileTable" (id, unified_file_id, model_mappings, updated_at)
VALUES ('f2', 'u2', '{}', '2026-01-01T00:00:00');
INSERT INTO "LiteLLM_ManagedObjectTable" (id, unified_object_id, model_object_id, file_object, file_purpose, updated_at)
VALUES ('o1', 'o', 'm', '{"id": "b", "completion_window": "24h", "created_at": 1, "endpoint": "/v1/chat/completions", "input_file_id": "f", "object": "batch", "status": "validating"}', 'batch', '2026-01-01T00:00:00');
INSERT INTO "LiteLLM_ManagedVectorStoreTable" (id, unified_resource_id, resource_object, model_mappings, updated_at)
VALUES ('v1', 'r1', '5', '{}', '2026-01-01T00:00:00');
INSERT INTO "LiteLLM_ManagedVectorStoresTable" (vector_store_id, custom_llm_provider, litellm_params, updated_at)
VALUES ('v1', 'openai', '{"index": "i"}', '2026-01-01T00:00:00');
INSERT INTO "LiteLLM_MCPServerTable" (server_id, mcp_access_groups)
VALUES ('s1', ARRAY[]::TEXT[]);
INSERT INTO "LiteLLM_AutoRouterSession" (api_key, session_id, router_name, router_type, first_turn_at, last_turn_at, last_model, turns, spend)
VALUES ('k', 'r1', 'router', 'complexity', '2026-01-01T00:00:00', '2026-01-01T00:00:00', 'm', 2, 0.5);
INSERT INTO "LiteLLM_SpendLogs" (request_id, call_type, api_key, startTime, endTime, model, metadata)
VALUES ('p1', 'completion', 'k', '2026-01-01T00:00:00', '2026-01-01T00:00:00', 'm', '{"trace": "t"}');
INSERT INTO "LiteLLM_ErrorLogs" (request_id, startTime, endTime, status_code, request_kwargs)
VALUES ('e1', '2026-01-01T00:00:00', '2026-01-01T00:00:00', '500', '{"error": "boom"}');
INSERT INTO "LiteLLM_TagTable" (tag_name) VALUES ('t1');
INSERT INTO "LiteLLM_SkillsTable" (skill_id, file_content, updated_at) VALUES ('k1', '\x610062', '2026-01-01T00:00:00');
"#;
