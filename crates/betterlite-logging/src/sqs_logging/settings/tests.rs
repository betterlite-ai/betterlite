#![cfg(test)]

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use crate::batch::logger::DEFAULT_MAX_QUEUE_SIZE;
use crate::sqs_logging::credentials::CredentialParams;
use crate::sqs_logging::encryption::EncryptionError;
use crate::sqs_logging::message::message_json;
use crate::sqs_logging::tests::support::{environment, payload};

use super::{
    DEFAULT_FLUSH_INTERVAL_SECONDS, DEFAULT_SQS_BATCH_SIZE, DEFAULT_STRIP_DEPTH,
    EncryptionOverrides, Overrides, Settings, SettingsError,
};

fn params(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn resolve(
    pairs: &[(&str, &str)],
    overrides: Overrides,
    variables: &[(&str, &str)],
) -> Result<Settings, SettingsError> {
    Settings::resolve(&params(pairs), overrides, &environment(variables))
}

fn empty_overrides() -> Overrides {
    Overrides::default()
}

fn key_b64() -> String {
    STANDARD.encode([7_u8; 32])
}

#[test]
fn resolves_the_connection_parameters_from_callback_settings() {
    let settings = resolve(
        &[
            ("sqs_queue_url", "https://queue.example.com"),
            ("sqs_region_name", "us-west-2"),
        ],
        Overrides::default(),
        &[],
    )
    .expect("the settings resolve");
    assert_eq!(
        settings.connection.queue_url.as_deref(),
        Some("https://queue.example.com")
    );
    assert_eq!(settings.connection.region.as_deref(), Some("us-west-2"));
}

#[test]
fn callback_settings_win_over_supplied_overrides() {
    let overrides = Overrides {
        queue_url: Some("https://override.example.com".to_owned()),
        region: Some("override-region".to_owned()),
        ..empty_overrides()
    };
    let settings = resolve(
        &[
            ("sqs_queue_url", "https://queue.example.com"),
            ("sqs_region_name", "us-west-2"),
        ],
        overrides,
        &[],
    )
    .expect("the settings resolve");
    assert_eq!(
        settings.connection.queue_url.as_deref(),
        Some("https://queue.example.com")
    );
    assert_eq!(settings.connection.region.as_deref(), Some("us-west-2"));
}

#[test]
fn supplied_overrides_fill_in_missing_callback_settings() {
    let overrides = Overrides {
        queue_url: Some("https://override.example.com".to_owned()),
        region: Some("override-region".to_owned()),
        ..empty_overrides()
    };
    let settings = resolve(&[], overrides, &[]).expect("the settings resolve");
    assert_eq!(
        settings.connection.queue_url.as_deref(),
        Some("https://override.example.com")
    );
    assert_eq!(
        settings.connection.region.as_deref(),
        Some("override-region")
    );
}

#[test]
fn defaults_match_litellm() {
    let settings = resolve(&[], Overrides::default(), &[]).expect("the settings resolve");
    assert!(settings.connection.queue_url.is_none());
    assert!(settings.connection.region.is_none());
    assert!(!settings.strip_base64_files);
    assert!(settings.encryption.is_none());
    assert_eq!(settings.strip_max_depth, DEFAULT_STRIP_DEPTH);
    assert_eq!(settings.batch.batch_size, DEFAULT_SQS_BATCH_SIZE);
    assert_eq!(
        settings.batch.flush_interval,
        Duration::from_secs(DEFAULT_FLUSH_INTERVAL_SECONDS)
    );
    assert_eq!(settings.batch.max_queue_size, DEFAULT_MAX_QUEUE_SIZE);
}

#[test]
fn environment_overrides_the_batch_size() {
    let settings = resolve(
        &[],
        Overrides::default(),
        &[("DEFAULT_SQS_BATCH_SIZE", "64")],
    )
    .expect("the settings resolve");
    assert_eq!(settings.batch.batch_size, 64);
}

#[test]
fn environment_overrides_the_flush_interval() {
    let settings = resolve(
        &[],
        Overrides::default(),
        &[("DEFAULT_SQS_FLUSH_INTERVAL_SECONDS", "2")],
    )
    .expect("the settings resolve");
    assert_eq!(settings.batch.flush_interval, Duration::from_secs(2));
}

#[test]
fn an_empty_environment_variable_takes_the_default() {
    let settings = resolve(
        &[],
        Overrides::default(),
        &[
            ("DEFAULT_SQS_BATCH_SIZE", ""),
            ("DEFAULT_SQS_FLUSH_INTERVAL_SECONDS", ""),
        ],
    )
    .expect("the settings resolve");
    assert_eq!(settings.batch.batch_size, DEFAULT_SQS_BATCH_SIZE);
    assert_eq!(
        settings.batch.flush_interval,
        Duration::from_secs(DEFAULT_FLUSH_INTERVAL_SECONDS)
    );
}

#[test]
fn an_invalid_environment_number_is_an_error() {
    let error = resolve(
        &[],
        Overrides::default(),
        &[("DEFAULT_SQS_BATCH_SIZE", "many")],
    )
    .err()
    .expect("the batch size is not a number");
    assert!(error.to_string().contains("DEFAULT_SQS_BATCH_SIZE"));
}

#[test]
fn the_strip_depth_is_read_from_the_environment() {
    let settings = resolve(
        &[],
        Overrides::default(),
        &[("DEFAULT_MAX_RECURSE_DEPTH_SENSITIVE_DATA_MASKER", "3")],
    )
    .expect("the settings resolve");
    assert_eq!(settings.strip_max_depth, 3);
}

#[test]
fn os_environ_reference_is_resolved() {
    let settings = resolve(
        &[("sqs_queue_url", "os.environ/SQS_QUEUE_URL")],
        Overrides::default(),
        &[("SQS_QUEUE_URL", "https://queue.example.com")],
    )
    .expect("the settings resolve");
    assert_eq!(
        settings.connection.queue_url.as_deref(),
        Some("https://queue.example.com")
    );
}

#[test]
fn a_missing_os_environ_reference_is_an_error() {
    let error = resolve(
        &[("sqs_queue_url", "os.environ/SQS_QUEUE_URL")],
        Overrides::default(),
        &[],
    )
    .err()
    .expect("the environment variable is missing");
    assert!(matches!(
        error,
        SettingsError::MissingEnvironment {
            name: "sqs_queue_url",
            variable
        } if variable == "SQS_QUEUE_URL"
    ));
}

#[test]
fn an_unknown_callback_setting_is_an_error() {
    let error = resolve(
        &[("sqs_unknown_setting", "value")],
        Overrides::default(),
        &[],
    )
    .err()
    .expect("the setting is not supported");
    assert!(matches!(
        error,
        SettingsError::UnknownKey { name } if name == "sqs_unknown_setting"
    ));
}

#[test]
fn an_invalid_bool_setting_is_an_error() {
    let error = resolve(
        &[("sqs_strip_base64_files", "sometimes")],
        Overrides::default(),
        &[],
    )
    .err()
    .expect("the setting is not a bool");
    assert!(matches!(
        error,
        SettingsError::Boolean {
            name: "sqs_strip_base64_files",
            ..
        }
    ));
}

#[test]
fn strip_base64_setting_is_parsed() {
    let settings = resolve(
        &[("sqs_strip_base64_files", "True")],
        Overrides::default(),
        &[],
    )
    .expect("the settings resolve");
    assert!(settings.strip_base64_files);
}

#[test]
fn encryption_requires_a_key() {
    let error = resolve(
        &[("sqs_aws_use_application_level_encryption", "True")],
        Overrides::default(),
        &[],
    )
    .err()
    .expect("the key is missing");
    assert!(matches!(error, SettingsError::MissingEncryptionKey));
}

#[test]
fn an_invalid_base64_key_is_an_error() {
    let error = resolve(
        &[
            ("sqs_aws_use_application_level_encryption", "True"),
            ("sqs_app_encryption_key_b64", "not base64!"),
        ],
        Overrides::default(),
        &[],
    )
    .err()
    .expect("the key is not base64");
    assert!(matches!(
        error,
        SettingsError::Encryption(EncryptionError::Decode(_))
    ));
}

#[test]
fn a_wrong_key_length_is_an_error() {
    let error = resolve(
        &[
            ("sqs_aws_use_application_level_encryption", "True"),
            ("sqs_app_encryption_key_b64", &STANDARD.encode(b"short")),
        ],
        Overrides::default(),
        &[],
    )
    .err()
    .expect("the key is not 32 bytes");
    assert!(matches!(
        error,
        SettingsError::Encryption(EncryptionError::Crypto(_))
    ));
}

#[test]
fn a_supplied_cipher_enables_encryption() {
    let overrides = Overrides {
        encryption: Some(EncryptionOverrides {
            key_b64: SecretString::from(key_b64()),
            aad: Some("tenant=bill".to_owned()),
        }),
        ..empty_overrides()
    };
    let settings = resolve(&[], overrides, &[]).expect("the settings resolve");
    let encryption = settings.encryption.expect("encryption is configured");
    let json = message_json(&payload(), Some(&encryption)).expect("the payload encrypts");
    assert!(json.contains("__encrypted__"));
}

#[test]
fn an_explicit_false_encryption_flag_wins_over_a_supplied_cipher() {
    let overrides = Overrides {
        encryption: Some(EncryptionOverrides {
            key_b64: SecretString::from(key_b64()),
            aad: None,
        }),
        ..empty_overrides()
    };
    let settings = resolve(
        &[("sqs_aws_use_application_level_encryption", "False")],
        overrides,
        &[],
    )
    .expect("the settings resolve");
    assert!(settings.encryption.is_none());
}

#[test]
fn credentials_are_read_from_callback_settings() {
    let settings = resolve(
        &[
            ("sqs_aws_access_key_id", "access"),
            ("sqs_aws_secret_access_key", "secret"),
            ("sqs_aws_session_token", "token"),
            ("sqs_aws_session_name", "session"),
            ("sqs_aws_profile_name", "profile"),
            ("sqs_aws_role_name", "role"),
            ("sqs_aws_web_identity_token", "identity"),
            ("sqs_aws_sts_endpoint", "https://sts.example.com"),
            ("sqs_region_name", "us-west-2"),
        ],
        Overrides::default(),
        &[],
    )
    .expect("the settings resolve");
    assert_credentials(&settings.credentials);
}

fn assert_credentials(credentials: &CredentialParams) {
    assert_eq!(
        credentials
            .access_key_id
            .as_ref()
            .expect("access key")
            .expose_secret(),
        "access"
    );
    assert_eq!(
        credentials
            .secret_access_key
            .as_ref()
            .expect("secret key")
            .expose_secret(),
        "secret"
    );
    assert_eq!(
        credentials
            .session_token
            .as_ref()
            .expect("session token")
            .expose_secret(),
        "token"
    );
    assert_eq!(credentials.session_name.as_deref(), Some("session"));
    assert_eq!(credentials.profile_name.as_deref(), Some("profile"));
    assert_eq!(credentials.role_name.as_deref(), Some("role"));
    assert_eq!(
        credentials
            .web_identity_token
            .as_ref()
            .expect("web identity token")
            .expose_secret(),
        "identity"
    );
    assert_eq!(
        credentials.sts_endpoint.as_deref(),
        Some("https://sts.example.com")
    );
    assert_eq!(credentials.region_name.as_deref(), Some("us-west-2"));
}

#[test]
fn batch_overrides_come_from_the_constructor_arguments() {
    let overrides = Overrides {
        batch_size: Some(7),
        flush_interval_seconds: Some(3),
        ..empty_overrides()
    };
    let settings = resolve(&[], overrides, &[]).expect("the settings resolve");
    assert_eq!(settings.batch.batch_size, 7);
    assert_eq!(settings.batch.flush_interval, Duration::from_secs(3));
}

#[rstest::rstest]
#[case::use_ssl("sqs_use_ssl", "False")]
#[case::verify("sqs_verify", "True")]
#[case::endpoint_url("sqs_endpoint_url", "http://localhost:4566")]
#[case::api_version("sqs_api_version", "2012-11-05")]
#[case::config("sqs_config", "{\"region\":\"local\"}")]
fn a_setting_the_sender_does_not_apply_is_refused(#[case] name: &str, #[case] value: &str) {
    let error = resolve(&[(name, value)], Overrides::default(), &[])
        .err()
        .expect("the setting has no effect, so it is refused");
    assert!(matches!(error, SettingsError::UnknownKey { name: refused } if refused == name));
}
