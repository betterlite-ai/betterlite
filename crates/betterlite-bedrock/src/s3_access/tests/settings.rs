use secrecy::ExposeSecret as _;

use crate::s3_access::settings::{
    S3Setting, S3SettingSources, SettingMap, SettingValue, resolve_s3_setting,
};

use super::support::{environment, secrets};

const KEY_PARAM: &str = "s3_encryption_key_id";
const OWNER_PARAM: &str = "s3_bucket_owner";
const KEY_ENV: &str = "AWS_S3_ENCRYPTION_KEY_ID";
const OWNER_ENV: &str = "AWS_S3_BUCKET_OWNER";

fn map(pairs: &[(&str, SettingValue)]) -> SettingMap {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn resolve(
    setting: S3Setting,
    params: &SettingMap,
    optional: Option<&SettingMap>,
    variables: &[(&str, &str)],
) -> Option<String> {
    let none = SettingMap::new();
    let lookup = secrets();
    let environment = environment(variables);
    let sources = S3SettingSources {
        params,
        optional_params: optional.unwrap_or(&none),
        secrets: &lookup,
        environment: &environment,
    };
    resolve_s3_setting(setting, &sources)
        .expect("the fixture lookup cannot fail")
        .map(|secret| secret.expose_secret().to_owned())
}

#[test]
fn a_parameter_beats_the_request_and_the_environment() {
    let configured = map(&[(KEY_PARAM, SettingValue::Text("from-params".to_owned()))]);
    let optional = map(&[(KEY_PARAM, SettingValue::Text("from-request".to_owned()))]);
    assert_eq!(
        resolve(
            S3Setting::EncryptionKeyId,
            &configured,
            Some(&optional),
            &[(KEY_ENV, "from-environment")]
        )
        .as_deref(),
        Some("from-params")
    );
}

#[test]
fn an_empty_or_non_text_parameter_falls_through_to_the_next_source() {
    let configured = map(&[
        (KEY_PARAM, SettingValue::Text(String::new())),
        (OWNER_PARAM, SettingValue::Other),
    ]);
    let optional = map(&[
        (KEY_PARAM, SettingValue::Text("from-request".to_owned())),
        (OWNER_PARAM, SettingValue::Text("owner-request".to_owned())),
    ]);
    assert_eq!(
        resolve(
            S3Setting::EncryptionKeyId,
            &configured,
            Some(&optional),
            &[(KEY_ENV, "from-environment")]
        )
        .as_deref(),
        Some("from-request")
    );
    assert_eq!(
        resolve(
            S3Setting::BucketOwner,
            &configured,
            Some(&optional),
            &[(OWNER_ENV, "owner-environment")]
        )
        .as_deref(),
        Some("owner-request")
    );
}

#[test]
fn the_environment_is_the_last_source_and_an_empty_value_counts_as_unset() {
    let configured = map(&[]);
    assert_eq!(
        resolve(
            S3Setting::EncryptionKeyId,
            &configured,
            None,
            &[(KEY_ENV, "from-environment")]
        )
        .as_deref(),
        Some("from-environment")
    );
    assert_eq!(
        resolve(
            S3Setting::EncryptionKeyId,
            &configured,
            None,
            &[(KEY_ENV, "")]
        ),
        None
    );
    assert_eq!(
        resolve(S3Setting::EncryptionKeyId, &configured, None, &[]),
        None
    );
    assert_eq!(
        resolve(
            S3Setting::BucketOwner,
            &configured,
            None,
            &[(KEY_ENV, "wrong-name")]
        ),
        None
    );
}

#[test]
fn whitespace_survives_and_syntax_is_not_validated() {
    let configured = map(&[(
        KEY_PARAM,
        SettingValue::Text(" key with spaces ".to_owned()),
    )]);
    assert_eq!(
        resolve(S3Setting::EncryptionKeyId, &configured, None, &[]).as_deref(),
        Some(" key with spaces ")
    );
    let empty = map(&[]);
    assert_eq!(
        resolve(
            S3Setting::BucketOwner,
            &empty,
            None,
            &[(OWNER_ENV, " 123456789012 ")]
        )
        .as_deref(),
        Some(" 123456789012 ")
    );
}

#[test]
fn each_setting_reads_only_its_own_parameter() {
    let configured = map(&[
        (KEY_PARAM, SettingValue::Text("encryption-key".to_owned())),
        (OWNER_PARAM, SettingValue::Text("123456789012".to_owned())),
    ]);
    assert_eq!(
        resolve(S3Setting::EncryptionKeyId, &configured, None, &[]).as_deref(),
        Some("encryption-key")
    );
    assert_eq!(
        resolve(S3Setting::BucketOwner, &configured, None, &[]).as_deref(),
        Some("123456789012")
    );
}
