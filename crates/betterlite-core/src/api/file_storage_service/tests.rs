#![cfg(test)]

use std::str;
use std::sync::Arc;

use betterlite_db::auxiliary_repository::file_content::ManagedFileContentRepository;
use betterlite_db::auxiliary_repository::repository::{PrismaClient, TableError};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::schema_budget_windows::MIGRATION_CATALOG;
use betterlite_db::schema_daily_usage::ADD_MANAGED_FILE_TABLE;
use betterlite_error::proxy_exception::ProxyException;
use betterlite_logging::azure_storage_logging::filename::IdSource;
use betterlite_openai::files::records::{FileKind, FileStatus, FilesPurpose};
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_redis::config_sync::publish::{PublishChange, PublishFuture};
use clap::Parser;
use deadpool_postgres::tokio_postgres::NoTls;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};

use crate::api::file_storage_contracts::storage::LITELLM_DB_BACKEND_NAME;
use crate::api::file_storage_service::error::{
    StorageBackendError, StorageServiceError, StorageUploadError,
};
use crate::api::file_storage_service::factory::get_storage_backend;
use crate::api::file_storage_service::projection::{
    FILE_ID_DIGITS, FILE_ID_PREFIX, FileMetadata, StoredFile,
    create_file_object_with_storage_metadata,
};
use crate::api::file_storage_service::service::{StorageUpload, upload_file_to_storage_backend};

mod support;

use support::{
    Captured, FixedClock, FixedIds, HexIds, Recorded, Recorder, backend_deps,
    backend_deps_without_environment, capture_logs, upload_deps, upload_deps_without_environment,
};

const FIXED_UUID: &str = "12345678-1234-5678-1234-567812345678";
const EXPECTED_FILE_ID: &str = "file-123456781234567812345678";
const AZURE_BLOB_PREFIX: &str = "https://acct.blob.core.windows.net/files/";
const AZURE_DFS_PREFIX: &str = "https://acct.dfs.core.windows.net/files/";
const UNSUPPORTED_OTHER: &str =
    "Unsupported storage backend type: other. Supported types: azure_storage, litellm_db";
const DATABASE_REQUIRED: &str = "Storage backend litellm_db requires a database-connected proxy";
const MISSING_ACCOUNT: &str = "Missing required environment variable: AZURE_STORAGE_ACCOUNT_NAME";
const TARGETS_UNAVAILABLE: &str = "Uploading with target_model_names requires a database-connected proxy, and this proxy has no database configured";
const CONTENT_MIGRATION: &str = "20260918000000_add_managed_file_content_table";
const MALFORMED_REFUSAL: &str = "the service raised a storage error";

fn upload_with<'data>(
    target_storage: &'data str,
    target_model_names: &'data [String],
) -> StorageUpload<'data> {
    StorageUpload {
        content: b"abc",
        filename: Some("a.txt"),
        content_type: Some("text/plain"),
        purpose: FilesPurpose::UserData,
        target_storage,
        target_model_names,
    }
}

fn default_upload(target_storage: &str) -> StorageUpload<'_> {
    StorageUpload {
        content: b"abc",
        filename: None,
        content_type: None,
        purpose: FilesPurpose::UserData,
        target_storage,
        target_model_names: &[],
    }
}

fn refused(error: StorageServiceError) -> ProxyException {
    match error {
        StorageServiceError::Proxy(proxy) => *proxy,
        StorageServiceError::Upload(_) => panic!("{MALFORMED_REFUSAL}"),
    }
}

fn assert_invalid_request(proxy: &ProxyException, message: &str, param: &str) {
    assert_eq!(proxy.message, message);
    assert_eq!(proxy.error_type, "invalid_request_error");
    assert_eq!(proxy.param.as_deref(), Some(param));
    assert_eq!(proxy.code, "400");
}

async fn refusal(
    target_storage: &str,
    targets: &[String],
    recorder: &Arc<Recorder>,
) -> ProxyException {
    let deps = upload_deps(recorder, None::<ManagedFileContentRepository<'static>>);
    let error = upload_file_to_storage_backend(upload_with(target_storage, targets), deps)
        .await
        .err()
        .expect("the upload is refused");
    refused(error)
}

async fn refusal_without_environment(
    recorder: &Arc<Recorder>,
    targets: &[String],
) -> ProxyException {
    let deps =
        upload_deps_without_environment(recorder, None::<ManagedFileContentRepository<'static>>);
    let error = upload_file_to_storage_backend(upload_with("azure_storage", targets), deps)
        .await
        .err()
        .expect("the Azure settings are missing");
    refused(error)
}

fn assert_uploaded_file(stored: &StoredFile, filename: &str, extension: &str) {
    assert_eq!(stored.file.id, EXPECTED_FILE_ID);
    assert_eq!(stored.file.bytes, 3);
    assert_eq!(stored.file.filename, filename);
    assert_eq!(stored.file.object, FileKind::File);
    assert_eq!(stored.file.purpose, FilesPurpose::UserData);
    assert_eq!(stored.file.status, Some(FileStatus::Uploaded));
    assert_eq!(stored.file.created_at, 1234);
    assert_eq!(stored.storage_backend, "azure_storage");
    let url = format!("{AZURE_BLOB_PREFIX}{FIXED_UUID}{extension}");
    assert_eq!(stored.storage_url, url);
}

fn assert_upload_requests(calls: &[Recorded], extension: &str) {
    let methods: Vec<&str> = calls.iter().map(|call| call.method.as_str()).collect();
    assert_eq!(methods, ["PUT", "PATCH", "PATCH"]);
    let base = format!("{AZURE_DFS_PREFIX}{FIXED_UUID}{extension}");
    let urls: Vec<&str> = calls.iter().map(|call| call.url.as_str()).collect();
    assert_eq!(
        urls,
        [
            format!("{base}?resource=file"),
            format!("{base}?action=append&position=0"),
            format!("{base}?action=flush&position=3"),
        ]
    );
    for call in calls {
        assert_eq!(call.header("authorization"), Some("Bearer tok-1"));
    }
    let append = calls.get(1).expect("the append request");
    assert_eq!(append.body, b"abc");
}

fn assert_upload_arguments() {
    let defaults = default_upload("azure_storage");
    assert_eq!(defaults.filename(), "file");
    assert_eq!(defaults.content_type(), "application/octet-stream");
    let explicit = upload_with("azure_storage", &[]);
    assert_eq!(explicit.filename(), "a.txt");
    assert_eq!(explicit.content_type(), "text/plain");
}

async fn default_upload_result() -> StoredFile {
    let recorder = Recorder::new(&[(201, ""), (202, ""), (200, "")]);
    let deps = upload_deps(&recorder, None::<ManagedFileContentRepository<'static>>);
    let stored = upload_file_to_storage_backend(default_upload("azure_storage"), deps)
        .await
        .expect("the storage-only upload");
    assert_upload_requests(&recorder.calls(), "");
    stored
}

async fn explicit_upload_result() -> StoredFile {
    let recorder = Recorder::new(&[(201, ""), (202, ""), (200, "")]);
    let deps = upload_deps(&recorder, None::<ManagedFileContentRepository<'static>>);
    let stored = upload_file_to_storage_backend(upload_with("azure_storage", &[]), deps)
        .await
        .expect("the explicit upload");
    assert_upload_requests(&recorder.calls(), ".txt");
    stored
}

async fn assert_storage_error() {
    let recorder = Recorder::new(&[(500, "")]);
    let deps = upload_deps(&recorder, None::<ManagedFileContentRepository<'static>>);
    let error = upload_file_to_storage_backend(upload_with("azure_storage", &[]), deps)
        .await
        .err()
        .expect("the storage error");
    assert!(matches!(
        error,
        StorageServiceError::Upload(boxed) if matches!(*boxed, StorageUploadError::Azure(_))
    ));
}

fn projected(
    content: &[u8],
    filename: &str,
    purpose: FilesPurpose,
    ids: &dyn IdSource,
) -> StoredFile {
    create_file_object_with_storage_metadata(
        FileMetadata {
            content,
            filename,
            purpose,
            target_storage: LITELLM_DB_BACKEND_NAME,
            storage_url: "litellm_db://r",
        },
        &FixedClock,
        ids,
    )
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn migrations() -> Vec<String> {
    let content = MIGRATION_CATALOG
        .files
        .iter()
        .find(|file| file.directory == CONTENT_MIGRATION)
        .expect("the content migration is in the catalog");
    let content_sql = str::from_utf8(content.bytes).expect("the content migration is UTF-8");
    let managed_sql =
        str::from_utf8(ADD_MANAGED_FILE_TABLE.bytes).expect("the managed file migration is UTF-8");
    vec![content_sql.to_owned(), managed_sql.to_owned()]
}

fn publisher() -> PublishChange<TableError> {
    Arc::new(|_object: &str| {
        let future: PublishFuture<TableError> = Box::pin(async { Ok::<(), TableError>(()) });
        future
    })
}

async fn provision() -> (PostgresFixture, PostgresStore) {
    let url = Environment::parse_from(["file-storage-service"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), migrations())
        .resolve()
        .await
        .expect("the database fixture");
    let address = fixture.options.url.expose_secret().parse();
    let manager = Manager::new(address.expect("the database url"), NoTls);
    let pool = Pool::builder(manager)
        .max_size(2)
        .build()
        .expect("the pool");
    (fixture, PostgresStore { pool: Some(pool) })
}

fn database_repository<'db>(
    store: &'db PostgresStore,
    availability: &'db Availability,
) -> ManagedFileContentRepository<'db> {
    ManagedFileContentRepository::new(
        Some(PrismaClient {
            writer: store,
            reader: store,
            availability,
        }),
        publisher(),
    )
}

async fn count(store: &PostgresStore, table: &str) -> i64 {
    let session = store.session().await.expect("a database session");
    let statement = format!("SELECT COUNT(*) FROM \"{table}\"");
    let row = session
        .query_one(statement.as_str(), &[])
        .await
        .expect("the row count");
    row.get(0)
}

async fn stored_content(store: &PostgresStore, row_id: &str) -> Vec<u8> {
    let session = store.session().await.expect("a database session");
    let row = session
        .query_one(
            "SELECT content FROM \"LiteLLM_ManagedFileContentTable\" WHERE id = $1",
            &[&row_id],
        )
        .await
        .expect("the stored content");
    row.get(0)
}

async fn assert_no_rows(store: &PostgresStore) {
    assert_eq!(count(store, "LiteLLM_ManagedFileContentTable").await, 0);
    assert_eq!(count(store, "LiteLLM_ManagedFileTable").await, 0);
}

async fn assert_database_rows(store: &PostgresStore, row_id: &str) {
    assert_eq!(count(store, "LiteLLM_ManagedFileContentTable").await, 1);
    assert_eq!(count(store, "LiteLLM_ManagedFileTable").await, 0);
    assert_eq!(stored_content(store, row_id).await, b"abc");
}

async fn close_database(fixture: PostgresFixture, store: PostgresStore) {
    store.pool.as_ref().expect("the pool").close();
    fixture.close_async().await.expect("the fixture closes");
}

fn assert_factory_logs(logs: &Captured) {
    let text = logs.text();
    for backend in ["azure_storage", "litellm_db", "other"] {
        assert!(
            text.contains(&format!("Creating storage backend: type={backend}")),
            "the factory logs {backend}"
        );
    }
}

#[tokio::test]
async fn storage_factory_errors() {
    let (logs, _guard) = capture_logs();
    let recorder = Recorder::new(&[]);
    let unsupported = get_storage_backend(
        "other",
        backend_deps(&recorder, None::<ManagedFileContentRepository<'static>>),
    )
    .err()
    .expect("the unsupported backend");
    assert_eq!(unsupported.to_string(), UNSUPPORTED_OTHER);
    let database = get_storage_backend(
        LITELLM_DB_BACKEND_NAME,
        backend_deps(&recorder, None::<ManagedFileContentRepository<'static>>),
    )
    .err()
    .expect("the missing database");
    assert_eq!(database.to_string(), DATABASE_REQUIRED);
    let no_settings =
        backend_deps_without_environment(&recorder, None::<ManagedFileContentRepository<'static>>);
    let settings = get_storage_backend("azure_storage", no_settings)
        .err()
        .expect("the Azure settings are missing");
    assert_eq!(settings.to_string(), MISSING_ACCOUNT);
    assert!(matches!(settings, StorageBackendError::Settings(_)));

    let other = refusal("other", &[], &recorder).await;
    assert_invalid_request(&other, UNSUPPORTED_OTHER, "target_storage");
    let missing = refusal(LITELLM_DB_BACKEND_NAME, &[], &recorder).await;
    assert_invalid_request(&missing, DATABASE_REQUIRED, "target_storage");
    let missing_settings = refusal_without_environment(&recorder, &[]).await;
    assert_invalid_request(&missing_settings, MISSING_ACCOUNT, "target_storage");
    assert!(recorder.calls().is_empty());
    assert_factory_logs(&logs);
}

#[tokio::test]
async fn storage_targets_before_write() {
    let recorder = Recorder::new(&[]);
    let targets = ["m".to_owned()];
    let missing_settings = refusal_without_environment(&recorder, &targets).await;
    assert_invalid_request(&missing_settings, MISSING_ACCOUNT, "target_storage");
    let invalid = refusal("other", &targets, &recorder).await;
    assert_invalid_request(&invalid, UNSUPPORTED_OTHER, "target_storage");

    let (fixture, store) = provision().await;
    let availability = Availability::default();
    let repository = database_repository(&store, &availability);
    let deps = upload_deps(&recorder, Some(repository));
    let failure = upload_file_to_storage_backend(upload_with("azure_storage", &targets), deps)
        .await
        .err()
        .expect("the managed files refusal");
    let proxy = refused(failure);
    assert_invalid_request(&proxy, TARGETS_UNAVAILABLE, "target_model_names");
    assert!(recorder.calls().is_empty());
    assert_no_rows(&store).await;
    close_database(fixture, store).await;
}

#[tokio::test]
async fn storage_only_upload() {
    let (logs, _guard) = capture_logs();
    assert_uploaded_file(&default_upload_result().await, "file", "");
    let completed = format!(
        "Storage backend upload complete: backend=azure_storage, url={AZURE_BLOB_PREFIX}{FIXED_UUID}"
    );
    assert!(logs.text().contains(&completed), "the upload debug line");
    assert_uploaded_file(&explicit_upload_result().await, "a.txt", ".txt");
    assert_upload_arguments();
    assert_storage_error().await;

    let (fixture, store) = provision().await;
    let availability = Availability::default();
    let repository = database_repository(&store, &availability);
    let recorder = Recorder::new(&[(201, ""), (202, ""), (200, "")]);
    let deps = upload_deps(&recorder, Some(repository));
    let stored = upload_file_to_storage_backend(default_upload("azure_storage"), deps)
        .await
        .expect("the connected storage-only upload");
    assert_uploaded_file(&stored, "file", "");
    assert_no_rows(&store).await;
    close_database(fixture, store).await;
}

#[test]
fn storage_file_projection() {
    let stored = projected(b"abc", "a.bin", FilesPurpose::Batch, &FixedIds);
    assert_eq!(stored.file.id, EXPECTED_FILE_ID);
    assert_eq!(stored.file.created_at, 1234);
    assert_eq!(stored.file.bytes, 3);
    assert_eq!(stored.file.filename, "a.bin");
    assert_eq!(stored.file.object, FileKind::File);
    assert_eq!(stored.file.purpose, FilesPurpose::Batch);
    assert_eq!(stored.file.status, Some(FileStatus::Uploaded));
    assert_eq!(stored.storage_backend, "litellm_db");
    assert_eq!(stored.storage_url, "litellm_db://r");
    let projection = serde_json::to_string(&stored.file).expect("the public file object");
    assert_eq!(
        projection,
        r#"{"id":"file-123456781234567812345678","bytes":3,"created_at":1234,"filename":"a.bin","object":"file","purpose":"batch","status":"uploaded"}"#
    );

    let empty = projected(b"", "empty.bin", FilesPurpose::UserData, &FixedIds);
    assert_eq!(empty.file.bytes, 0);
    assert_eq!(empty.file.id, stored.file.id);

    let hex = projected(b"abc", "a.bin", FilesPurpose::Batch, &HexIds);
    assert_eq!(hex.file.id, "file-abcdef1234567890abcdef12");
}

#[tokio::test]
async fn managed_storage_unavailable() {
    let recorder = Recorder::new(&[]);
    let targets = ["m".to_owned()];
    let deps = upload_deps(&recorder, None::<ManagedFileContentRepository<'static>>);
    let failure = upload_file_to_storage_backend(upload_with("azure_storage", &targets), deps)
        .await
        .err()
        .expect("the managed files refusal");
    let proxy = refused(failure);
    assert_invalid_request(&proxy, TARGETS_UNAVAILABLE, "target_model_names");
    assert!(recorder.calls().is_empty());
}

#[tokio::test]
async fn database_storage_only() {
    let (fixture, store) = provision().await;
    let availability = Availability::default();
    let repository = database_repository(&store, &availability);
    let recorder = Recorder::new(&[]);
    let deps = upload_deps(&recorder, Some(repository));
    let stored = upload_file_to_storage_backend(upload_with(LITELLM_DB_BACKEND_NAME, &[]), deps)
        .await
        .expect("the database upload");
    let digits = stored
        .file
        .id
        .strip_prefix(FILE_ID_PREFIX)
        .expect("the file id prefix");
    assert_eq!(digits.len(), FILE_ID_DIGITS);
    assert_eq!(stored.file.bytes, 3);
    assert_eq!(stored.file.purpose, FilesPurpose::UserData);
    assert_eq!(stored.storage_backend, LITELLM_DB_BACKEND_NAME);
    let row_id = stored
        .storage_url
        .strip_prefix("litellm_db://")
        .expect("the database storage url");
    assert_database_rows(&store, row_id).await;
    assert!(recorder.calls().is_empty());
    close_database(fixture, store).await;
}
