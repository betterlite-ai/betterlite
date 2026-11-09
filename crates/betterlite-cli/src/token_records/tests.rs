#![cfg(test)]

use std::error::Error as _;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::private_json::files::PrivateFiles;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tempfile::TempDir;

use super::commit::commit_token_file;
use super::error::TokenRecordError;
use super::file::{read_token_file, token_file_path};
use super::freshness::is_token_fresh;
use super::overwrite::overwrite_token_file_secret;
use super::record::{CliTokenRecord, holds_secret, without_secret};
use super::secret::{decode_secret, encode_secret};
use super::stage::stage_token_file;
use super::write::write_token_file;

const RECORD_JSON: &str = r#"{"base_url":"https://gateway.example","key":"sk-secret","user_id":"user-1","user_email":"user@example.com","user_role":"admin","auth_header_name":"X-Custom-Authorization","jwt_token":"jwt-secret","timestamp":1762700000.5,"expires_at":1762703600.5,"refresh_token":"refresh-secret","client_id":"client-1","token_endpoint":"https://gateway.example/token","revocation_endpoint":"https://gateway.example/revoke","resource":"https://gateway.example/v1","team_id":"team-1"}"#;
const DEFAULT_RECORD_JSON: &str = r#"{"base_url":"","user_id":"","user_email":"","user_role":"","auth_header_name":"Authorization","jwt_token":"","timestamp":0.0}"#;
const STRIPPED_RECORD_JSON: &str = r#"{"base_url":"https://gateway.example","user_id":"user-1","user_email":"user@example.com","user_role":"admin","auth_header_name":"X-Custom-Authorization","jwt_token":"","timestamp":1762700000.5,"expires_at":1762703600.5,"client_id":"client-1","token_endpoint":"https://gateway.example/token","revocation_endpoint":"https://gateway.example/revoke","resource":"https://gateway.example/v1","team_id":"team-1"}"#;

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn record() -> CliTokenRecord {
    CliTokenRecord {
        base_url: "https://gateway.example".to_owned(),
        key: Some(secret("sk-secret")),
        user_id: "user-1".to_owned(),
        user_email: "user@example.com".to_owned(),
        user_role: "admin".to_owned(),
        auth_header_name: "X-Custom-Authorization".to_owned(),
        jwt_token: secret("jwt-secret"),
        timestamp: 1_762_700_000.5,
        expires_at: Some(1_762_703_600.5),
        refresh_token: Some(secret("refresh-secret")),
        client_id: Some("client-1".to_owned()),
        token_endpoint: Some("https://gateway.example/token".to_owned()),
        revocation_endpoint: Some("https://gateway.example/revoke".to_owned()),
        resource: Some("https://gateway.example/v1".to_owned()),
        team_id: Some("team-1".to_owned()),
    }
}

fn record_at(timestamp: f64, expires_at: Option<f64>) -> CliTokenRecord {
    let mut record = record();
    record.timestamp = timestamp;
    record.expires_at = expires_at;
    record
}

fn token_path(directory: &TempDir) -> PathBuf {
    token_file_path(directory.path())
}

fn mode_of(path: &Path) -> u32 {
    fs::metadata(path)
        .expect("reads the metadata")
        .permissions()
        .mode()
        & 0o777
}

fn assert_io_source(error: &TokenRecordError) {
    let source = error.source().expect("a wrapped source");
    let private = source
        .downcast_ref::<PrivateJsonError>()
        .expect("the private JSON error");
    let io_source = private.source().expect("a wrapped I/O error");
    assert!(io_source.is::<io::Error>());
}

#[test]
fn parses_declared_and_pass_through_fields() {
    let text = r#"{
        "base_url": "https://gateway.example",
        "key": "sk-secret",
        "user_id": "user-1",
        "user_email": "user@example.com",
        "user_role": "admin",
        "auth_header_name": "X-Custom-Authorization",
        "jwt_token": "jwt-secret",
        "timestamp": 1762700000.5,
        "expires_at": 1762703600.5,
        "refresh_token": "refresh-secret",
        "client_id": "client-1",
        "token_endpoint": "https://gateway.example/token",
        "revocation_endpoint": "https://gateway.example/revoke",
        "resource": "https://gateway.example/v1",
        "team_id": "team-1"
    }"#;
    let record: CliTokenRecord = serde_json::from_str(text).expect("parses the stored credential");
    assert_eq!(
        serde_json::to_string(&record).expect("serializes the parsed record"),
        RECORD_JSON
    );
}

#[test]
fn parses_a_record_with_defaults() {
    let parsed: CliTokenRecord = serde_json::from_str("{}").expect("parses an empty credential");
    assert_eq!(
        serde_json::to_string(&parsed).expect("serializes the parsed record"),
        DEFAULT_RECORD_JSON
    );
    assert_eq!(
        serde_json::to_string(&CliTokenRecord::default()).expect("serializes the default record"),
        DEFAULT_RECORD_JSON
    );
    let with_null: CliTokenRecord =
        serde_json::from_str(r#"{"team_id": null}"#).expect("parses a null team");
    assert_eq!(
        serde_json::to_string(&with_null).expect("serializes the null team record"),
        DEFAULT_RECORD_JSON
    );
}

#[test]
fn round_trips_through_encode_and_decode_secret() {
    let record = record();
    let blob = encode_secret(&record).expect("encodes the secret");
    assert_eq!(
        blob.expose_secret(),
        r#"{"base_url":"https://gateway.example","key":"sk-secret","jwt_token":"jwt-secret","refresh_token":"refresh-secret","timestamp":1762700000.5}"#
    );
    let secret = decode_secret(blob.expose_secret(), &record.base_url).expect("decodes the secret");
    assert_eq!(secret.base_url, record.base_url);
    assert_eq!(
        secret.key.as_ref().expect("key").expose_secret(),
        "sk-secret"
    );
    assert_eq!(secret.jwt_token.expose_secret(), "jwt-secret");
    assert_eq!(
        secret
            .refresh_token
            .as_ref()
            .expect("refresh token")
            .expose_secret(),
        "refresh-secret"
    );
    assert_eq!(secret.timestamp.to_bits(), record.timestamp.to_bits());

    let mut bare = record;
    bare.key = None;
    bare.jwt_token = SecretString::default();
    bare.refresh_token = None;
    let bare_blob = encode_secret(&bare).expect("encodes the bare secret");
    assert_eq!(
        bare_blob.expose_secret(),
        r#"{"base_url":"https://gateway.example","key":null,"jwt_token":"","refresh_token":null,"timestamp":1762700000.5}"#
    );
}

#[test]
fn decode_secret_rejects_mismatched_base_url() {
    let blob = encode_secret(&record()).expect("encodes the secret");
    assert!(decode_secret(blob.expose_secret(), "https://other.example").is_none());
    assert!(decode_secret(blob.expose_secret(), "https://gateway.example").is_some());
}

#[test]
fn decode_secret_returns_nothing_for_invalid_json() {
    assert!(decode_secret("not json", "https://gateway.example").is_none());
    assert!(decode_secret(r#"{"key":"sk-secret"}"#, "https://gateway.example").is_none());
    assert!(decode_secret(r#"{"base_url":7}"#, "https://gateway.example").is_none());
}

#[test]
fn encode_secret_writes_a_non_finite_timestamp_as_null() {
    let mut record = record();
    record.timestamp = f64::NAN;
    let blob = encode_secret(&record).expect("encodes the secret");
    assert!(blob.expose_secret().contains(r#""timestamp":null"#));
    assert!(decode_secret(blob.expose_secret(), &record.base_url).is_none());
}

#[rstest]
#[case::nothing(None, "", None, false)]
#[case::key(Some("sk-secret"), "", None, true)]
#[case::refresh_token(None, "", Some("refresh-secret"), true)]
#[case::jwt_token(None, "jwt-secret", None, true)]
fn holds_secret_true_only_when_key_jwt_or_refresh_present(
    #[case] key: Option<&str>,
    #[case] jwt_token: &str,
    #[case] refresh_token: Option<&str>,
    #[case] expected: bool,
) {
    let mut record = record_at(0.0_f64, None);
    record.key = key.map(secret);
    record.jwt_token = secret(jwt_token);
    record.refresh_token = refresh_token.map(secret);
    assert_eq!(holds_secret(&record), expected);
}

#[test]
fn without_secret_clears_only_the_three_secret_fields() {
    let record = record();
    let stripped = without_secret(&record);
    assert!(stripped.key.is_none());
    assert_eq!(stripped.jwt_token.expose_secret(), "");
    assert!(stripped.refresh_token.is_none());
    assert_eq!(
        serde_json::to_string(&stripped).expect("serializes the stripped record"),
        STRIPPED_RECORD_JSON
    );
    assert_eq!(
        record.key.as_ref().expect("key").expose_secret(),
        "sk-secret"
    );
}

#[test]
fn fresh_before_expires_at_minus_buffer() {
    let record = record_at(0.0_f64, Some(1_000.0_f64));
    assert!(is_token_fresh(&record, 639.0, 24));
    assert!(is_token_fresh(&record, 0.0, 24));
}

#[test]
fn stale_at_expires_at_minus_buffer() {
    let record = record_at(0.0_f64, Some(1_000.0_f64));
    assert!(!is_token_fresh(&record, 640.0, 24));
    assert!(!is_token_fresh(&record, 1_000.0, 24));
    assert!(!is_token_fresh(&record, 2_000.0, 10_000));
}

#[test]
fn fresh_falls_back_to_timestamp_age() {
    let record = record_at(1_000_000.0_f64, None);
    assert!(is_token_fresh(&record, 1_086_039.0, 24));
    let future = record_at(10_000.0_f64, None);
    assert!(is_token_fresh(&future, 5_000.0, 0));
}

#[test]
fn stale_at_expiration_hours_minus_buffer() {
    let record = record_at(1_000_000.0_f64, None);
    assert!(!is_token_fresh(&record, 1_086_040.0, 24));
    let epoch = record_at(0.0_f64, None);
    assert!(!is_token_fresh(&epoch, 0.0, 0));
}

#[test]
fn token_file_path_joins_the_home_directory() {
    assert_eq!(
        token_file_path(Path::new("/home/user")),
        PathBuf::from("/home/user/.litellm/token.json")
    );
}

#[test]
fn read_token_file_none_on_missing_file() {
    let directory = TempDir::new().expect("temp dir");
    assert!(read_token_file(&token_path(&directory)).is_none());
}

#[test]
fn read_token_file_none_on_unreadable_path() {
    let directory = TempDir::new().expect("temp dir");
    assert!(read_token_file(directory.path()).is_none());
}

#[test]
fn read_token_file_none_on_corrupt_json() {
    let directory = TempDir::new().expect("temp dir");
    let path = token_path(&directory);
    fs::create_dir_all(path.parent().expect("parent")).expect("creates the directory");
    fs::write(&path, "not json").expect("writes the corrupt file");
    assert!(read_token_file(&path).is_none());
    fs::write(&path, r#"{"timestamp": "soon"}"#).expect("writes the wrong type");
    assert!(read_token_file(&path).is_none());
    fs::write(&path, r#"{"key": 7}"#).expect("writes the wrong key type");
    assert!(read_token_file(&path).is_none());
}

#[test]
fn read_token_file_parses_a_valid_record() {
    let directory = TempDir::new().expect("temp dir");
    let path = token_path(&directory);
    fs::create_dir_all(path.parent().expect("parent")).expect("creates the directory");
    fs::write(&path, RECORD_JSON).expect("writes the record");
    let stored = read_token_file(&path).expect("reads the record");
    assert_eq!(
        stored.key.as_ref().expect("key").expose_secret(),
        "sk-secret"
    );
    assert_eq!(stored.team_id.as_deref(), Some("team-1"));
}

#[tokio::test]
async fn write_token_file_sets_owner_only_permissions() {
    let directory = TempDir::new().expect("temp dir");
    let path = token_path(&directory);
    let config_dir = path.parent().expect("parent").to_owned();
    fs::create_dir_all(&config_dir).expect("creates the directory");
    fs::set_permissions(&config_dir, fs::Permissions::from_mode(0o755))
        .expect("widens the directory");
    let files = PrivateFiles::default();
    write_token_file(&files, &path, &record())
        .await
        .expect("writes the token file");
    assert_eq!(mode_of(&config_dir), 0o700);
    assert_eq!(mode_of(&path), 0o600);
    let stored = read_token_file(&path).expect("reads the record");
    assert_eq!(stored.team_id.as_deref(), Some("team-1"));
}

#[tokio::test]
async fn stage_and_commit_token_file_replaces_existing_content() {
    let directory = TempDir::new().expect("temp dir");
    let path = token_path(&directory);
    let files = PrivateFiles::default();
    let mut original = record();
    original.user_id = "original-user".to_owned();
    write_token_file(&files, &path, &original)
        .await
        .expect("writes the original");
    let mut replacement = record();
    replacement.user_id = "replacement-user".to_owned();
    let staged = stage_token_file(&files, &path, &replacement)
        .await
        .expect("stages the replacement");
    assert!(staged.path.exists());
    assert_eq!(
        read_token_file(&path).expect("reads the original").user_id,
        "original-user"
    );
    commit_token_file(&files, &staged, &path)
        .await
        .expect("commits the replacement");
    assert!(!staged.path.exists());
    assert_eq!(
        read_token_file(&path)
            .expect("reads the replacement")
            .user_id,
        "replacement-user"
    );
    assert_eq!(mode_of(&path), 0o600);
}

#[tokio::test]
async fn overwrite_token_file_secret_strips_secret_in_place() {
    let directory = TempDir::new().expect("temp dir");
    let path = token_path(&directory);
    let files = PrivateFiles::default();
    let record = record();
    write_token_file(&files, &path, &record)
        .await
        .expect("writes the record");
    overwrite_token_file_secret(&files, &path, &record)
        .await
        .expect("strips the secret");
    let text = fs::read_to_string(&path).expect("reads the stripped file");
    assert!(!text.contains("sk-secret"));
    assert!(!text.contains("jwt-secret"));
    assert!(!text.contains("refresh-secret"));
    let stored = read_token_file(&path).expect("reads the stripped record");
    assert!(stored.key.is_none());
    assert_eq!(stored.jwt_token.expose_secret(), "");
    assert!(stored.refresh_token.is_none());
    assert_eq!(stored.team_id.as_deref(), Some("team-1"));
    assert_eq!(mode_of(&path), 0o600);
}

#[tokio::test]
async fn stage_token_file_returns_its_own_error_with_the_io_source() {
    let directory = TempDir::new().expect("temp dir");
    let blocker = directory.path().join("blocker");
    fs::write(&blocker, "not a directory").expect("writes the blocker");
    let path = blocker.join("token.json");
    let files = PrivateFiles::default();
    let error = stage_token_file(&files, &path, &record())
        .await
        .expect_err("staging fails");
    assert!(matches!(error, TokenRecordError::Directory { .. }));
    assert_io_source(&error);
}

#[tokio::test]
async fn commit_token_file_returns_its_own_error_with_the_io_source() {
    let directory = TempDir::new().expect("temp dir");
    let source_path = directory.path().join("source.json");
    let files = PrivateFiles::default();
    let staged = stage_token_file(&files, &source_path, &record())
        .await
        .expect("stages the record");
    let blocker = directory.path().join("blocker");
    fs::write(&blocker, "not a directory").expect("writes the blocker");
    let path = blocker.join("token.json");
    let error = commit_token_file(&files, &staged, &path)
        .await
        .expect_err("committing fails");
    assert!(matches!(error, TokenRecordError::Commit { .. }));
    assert_io_source(&error);
}

#[tokio::test]
async fn write_token_file_returns_its_own_error_with_the_io_source() {
    let directory = TempDir::new().expect("temp dir");
    let blocker = directory.path().join("blocker");
    fs::write(&blocker, "not a directory").expect("writes the blocker");
    let path = blocker.join("token.json");
    let files = PrivateFiles::default();
    let error = write_token_file(&files, &path, &record())
        .await
        .expect_err("writing fails");
    assert!(matches!(error, TokenRecordError::Directory { .. }));
    assert_io_source(&error);
}

#[tokio::test]
async fn overwrite_token_file_secret_returns_its_own_error_with_the_io_source() {
    let directory = TempDir::new().expect("temp dir");
    let path = token_path(&directory);
    let files = PrivateFiles::default();
    let error = overwrite_token_file_secret(&files, &path, &record())
        .await
        .expect_err("overwriting fails");
    assert!(matches!(error, TokenRecordError::Overwrite { .. }));
    assert_io_source(&error);
}
