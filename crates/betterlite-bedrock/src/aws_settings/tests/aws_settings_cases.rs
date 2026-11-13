#![cfg(test)]

use betterlite_openai::error::{ErrorRequest, ErrorResponse};

use crate::aws_settings::{
    error::{AWS_CONSOLE_URL, AwsAuthError},
    policy::web_identity_policy,
};
use std::collections::BTreeMap;
use std::future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::yield_now;

use super::support::{cache, credentials, native, settings};
use crate::aws_settings::credential_cache::{Parameter, cache_key};
use crate::aws_settings::sources::{
    SourceError, auth_selector_presence, resolve_local_credentials,
};
use crate::aws_settings::{
    assume_role_parameters, bedrock_bearer_token, canonicalize_tags, pop_aws_auth_params,
    s3_static_key_pair,
};

#[derive(Clone, Copy)]
enum Case {
    Pair,
    Session,
    Profile,
    Ambient,
    Tags,
    Expiry,
    Bearer,
    Pop,
    Role,
    Policy,
    InvalidPop,
    Bound,
    Sources,
    Selectors,
    Error,
}

#[rstest]
#[case::static_key_pair(Case::Pair)]
#[case::session_token_triple(Case::Session)]
#[case::profile_precedence(Case::Profile)]
#[case::default_native_chain(Case::Ambient)]
#[case::canonical_session_tags(Case::Tags)]
#[case::credential_cache_expiry(Case::Expiry)]
#[case::bearer_token_empty_versus_absent(Case::Bearer)]
#[case::auth_params_removed_from_body(Case::Pop)]
#[case::session_tags_canonical(Case::Tags)]
#[case::assume_role_parameter_presence(Case::Role)]
#[case::web_identity_policy(Case::Policy)]
#[case::auth_parameters_popped_before_validation(Case::InvalidPop)]
#[case::bearer_presence_not_truthiness(Case::Bearer)]
#[case::credential_cache_key_and_bound(Case::Bound)]
#[case::local_credential_sources(Case::Sources)]
#[case::auth_selector_presence(Case::Selectors)]
#[case::aws_auth_error_projection(Case::Error)]
#[tokio::test]
async fn aws_settings_cases(#[case] case: Case) {
    match case {
        Case::Pair => pair(),
        Case::Session => session().await,
        Case::Profile | Case::Ambient => discovery(case).await,
        Case::Tags => tags(),
        Case::Expiry => expiry().await,
        Case::Bearer => bearer(),
        Case::Pop => pop(),
        Case::Role => role(),
        Case::Policy => policy(),
        Case::InvalidPop => invalid_pop(),
        Case::Bound => bound().await,
        Case::Sources => sources().await,
        Case::Selectors => selectors(),
        Case::Error => error(),
    }
}

fn pair() {
    use betterlite_config::sections::bedrock_aws_settings::AwsS3Settings;
    let input = AwsS3Settings {
        access_key_id: Some(SecretString::from(" ".to_owned())),
        secret_access_key: Some(SecretString::from("secret".to_owned())),
    };
    let (access, secret) = s3_static_key_pair(&input).unwrap();
    assert_eq!(
        (access.expose_secret(), secret.expose_secret()),
        (" ", "secret")
    );
    assert!(
        s3_static_key_pair(&AwsS3Settings {
            access_key_id: Some(SecretString::from(String::new())),
            secret_access_key: input.secret_access_key
        })
        .is_none()
    );
    assert!(s3_static_key_pair(&AwsS3Settings::default()).is_none());
}

async fn session() {
    let input =
        settings("aws_access_key_id: '', aws_secret_access_key: secret, aws_session_token: token");
    let resolved = resolve_local_credentials(&input, &native(&[], &[]))
        .await
        .unwrap();
    assert!(resolved.cache_ttl.is_none());
    let result = resolved.credentials;
    assert_eq!(result.access_key_id.expose_secret(), "");
    assert_eq!(result.secret_access_key.expose_secret(), "secret");
    assert_eq!(result.session_token.unwrap().expose_secret(), "token");
}

async fn discovery(case: Case) {
    let (input, files, expected) = if matches!(case, Case::Profile) {
        (
            settings(
                "aws_profile_name: chosen, aws_access_key_id: ignored, aws_secret_access_key: ignored, aws_session_token: ignored",
            ),
            vec![(
                "/credentials",
                "[chosen]\naws_access_key_id = profile\naws_secret_access_key = private\n",
            )],
            "profile",
        )
    } else {
        (settings(""), Vec::new(), "ambient")
    };
    let provider = native(
        &files,
        &[
            ("AWS_SHARED_CREDENTIALS_FILE", "/credentials"),
            ("AWS_ACCESS_KEY_ID", "ambient"),
            ("AWS_SECRET_ACCESS_KEY", "private"),
            ("AWS_EC2_METADATA_DISABLED", "true"),
        ],
    );
    let result = resolve_local_credentials(&input, &provider).await.unwrap();
    assert!(result.cache_ttl.is_none());
    assert_eq!(result.credentials.access_key_id.expose_secret(), expected);
}

fn tags() {
    let raw = serde_json::from_str(r#"[{"Key":"z","Value":"last"},{"Key":"a","Value":"first","extra":true},{"Key":"a","Value":"second"},{"Key":"","Value":""}]"#).unwrap();
    let result = canonicalize_tags(Some(raw)).unwrap().unwrap();
    assert_eq!(
        result
            .iter()
            .map(|tag| (tag.key.as_str(), tag.value.as_str()))
            .collect::<Vec<_>>(),
        [("", ""), ("a", "first"), ("a", "second"), ("z", "last")]
    );
    assert!(canonicalize_tags(None).unwrap().is_none());
    drop(canonicalize_tags(Some(serde_json::from_str("42").unwrap())).unwrap_err());
}

async fn expiry() {
    let clock = Arc::new(AtomicU32::new(0));
    let storage = cache(Arc::clone(&clock));
    let params = BTreeMap::new();
    storage
        .get_or_fetch(&params, || async { Ok(credentials("first")) })
        .await
        .unwrap();
    clock.store(600, Ordering::SeqCst);
    let hit = storage
        .get_or_fetch(&params, || async { Err(SourceError::MissingProvider) })
        .await
        .unwrap();
    assert_eq!(hit.access_key_id.expose_secret(), "first");
    clock.store(601, Ordering::SeqCst);
    let refreshed = storage
        .get_or_fetch(&params, || async { Ok(credentials("next")) })
        .await
        .unwrap();
    assert_eq!(refreshed.access_key_id.expose_secret(), "next");
}

fn bearer() {
    let configured = SecretString::from("configured".to_owned());
    let empty = SecretString::from(String::new());
    assert!(bedrock_bearer_token(Some(&empty), Some(&configured)).is_none());
    assert_eq!(
        bedrock_bearer_token(None, Some(&configured))
            .unwrap()
            .expose_secret(),
        "configured"
    );
    let whitespace = SecretString::from(" ".to_owned());
    assert_eq!(
        bedrock_bearer_token(Some(&whitespace), None)
            .unwrap()
            .expose_secret(),
        " "
    );
}

fn pop() {
    let mut params = serde_json::from_str(r#"{"aws_access_key_id":"access","aws_region_name":"west","aws_bedrock_runtime_endpoint":"endpoint"}"#).unwrap();
    let auth = pop_aws_auth_params(&mut params).unwrap();
    assert_eq!(auth.aws_access_key_id.unwrap().expose_secret(), "access");
    assert!(auth.aws_profile_name.is_none());
    assert_eq!(
        params.keys().map(String::as_str).collect::<Vec<_>>(),
        ["aws_bedrock_runtime_endpoint", "aws_region_name"]
    );
}

fn role() {
    let parameters = assume_role_parameters(
        String::new(),
        String::new(),
        Some(String::new()),
        Some(Vec::new()),
    );
    let rendered = serde_json::to_string(&parameters).unwrap();
    assert_eq!(
        rendered,
        r#"{"RoleArn":"","RoleSessionName":"","ExternalId":""}"#
    );
    let absent = assume_role_parameters("arn".to_owned(), "session".to_owned(), None, None);
    assert!(
        !serde_json::to_string(&absent)
            .unwrap()
            .contains("ExternalId")
    );
}

fn policy() {
    let value = web_identity_policy();
    assert_eq!(value.version, "2012-10-17");
    assert_eq!(
        value
            .statement
            .iter()
            .map(|item| item.sid)
            .collect::<Vec<_>>(),
        [
            "BedrockLiteLLM",
            "BedrockAgentCoreLiteLLM",
            "ClaudePlatformLiteLLM",
            "BedrockMantleLiteLLM"
        ]
    );
    assert_eq!(
        value
            .statement
            .iter()
            .map(|item| item.action.len())
            .collect::<Vec<_>>(),
        [10, 3, 7, 1]
    );
    assert!(value.statement.iter().all(|item| item.effect == "Allow"
        && item.resource == "*"
        && item.condition.secure.enabled == "true"));
}

fn invalid_pop() {
    let mut params = serde_json::from_str(r#"{"aws_access_key_id":42,"aws_secret_access_key":"secret","aws_session_tags":"invalid","aws_region_name":"west"}"#).unwrap();
    assert!(
        pop_aws_auth_params(&mut params)
            .unwrap_err()
            .to_string()
            .contains("aws_access_key_id")
    );
    assert_eq!(
        params.keys().map(String::as_str).collect::<Vec<_>>(),
        ["aws_region_name"]
    );
}

async fn bound() {
    let storage = cache(Arc::new(AtomicU32::new(0)));
    for index in 0_u32..1001 {
        let params = BTreeMap::from([(
            "identity".to_owned(),
            Parameter::Text(SecretString::from(index.to_string())),
        )]);
        storage
            .get_or_fetch(&params, || async { Ok(credentials("bounded")) })
            .await
            .unwrap();
    }
    let mut misses = 0_u32;
    for index in 0_u32..1001 {
        let params = BTreeMap::from([(
            "identity".to_owned(),
            Parameter::Text(SecretString::from(index.to_string())),
        )]);
        if storage
            .get_or_fetch(&params, || async { Err(SourceError::MissingProvider) })
            .await
            .is_err()
        {
            misses = misses.saturating_add(1);
        }
    }
    assert_eq!(misses, 1);
}

async fn sources() {
    let input = settings(
        "aws_access_key_id: static, aws_secret_access_key: private, aws_region_name: west",
    );
    let result = resolve_local_credentials(&input, &native(&[], &[]))
        .await
        .unwrap();
    assert_eq!(result.cache_ttl, Some(Duration::from_mins(59)));
    assert_eq!(result.credentials.access_key_id.expose_secret(), "static");
    let provider = native(
        &[(
            "/credentials",
            "[broken]\ncredential_process = must-never-run\n",
        )],
        &[
            ("AWS_SHARED_CREDENTIALS_FILE", "/credentials"),
            ("AWS_EC2_METADATA_DISABLED", "true"),
        ],
    );
    drop(
        resolve_local_credentials(&settings("aws_profile_name: broken"), &provider)
            .await
            .unwrap_err(),
    );
}

fn selectors() {
    let empty = settings(
        "aws_access_key_id: '', aws_secret_access_key: '', aws_session_token: '', aws_region_name: '', aws_profile_name: '', aws_role_name: '', aws_web_identity_token: '', aws_session_name: ''",
    );
    assert_eq!(auth_selector_presence(&empty), [true; 5]);
    assert_eq!(auth_selector_presence(&settings("")), [false; 5]);
}

fn error() {
    let request = ErrorRequest {
        method: "POST".to_owned(),
        url: AWS_CONSOLE_URL.to_owned(),
    };
    let error = AwsAuthError {
        status: 403,
        message: "denied".to_owned(),
        response: ErrorResponse {
            status_code: 403,
            request: request.clone(),
        },
        request,
    };
    assert_eq!((error.status, error.message.as_str()), (403, "denied"));
    assert_eq!(error.request.method, "POST");
    assert_eq!(
        error.request.url,
        "https://us-west-2.console.aws.amazon.com/bedrock"
    );
    assert_eq!(error.request, error.response.request);
    assert_eq!(error.response.status_code, 403);
}

#[tokio::test]
async fn bedrock_aws_settings_credential_identity_isolation() {
    let storage = cache(Arc::new(AtomicU32::new(0)));
    let first = BTreeMap::from([(
        "aws_session_name".to_owned(),
        Parameter::Text(SecretString::from("one".to_owned())),
    )]);
    let second = BTreeMap::from([(
        "aws_session_name".to_owned(),
        Parameter::Text(SecretString::from("two".to_owned())),
    )]);
    assert_ne!(cache_key(&first).unwrap(), cache_key(&second).unwrap());
    storage
        .get_or_fetch(&first, || async { Ok(credentials("one")) })
        .await
        .unwrap();
    let result = storage
        .get_or_fetch(&second, || async { Ok(credentials("two")) })
        .await
        .unwrap();
    assert_eq!(result.access_key_id.expose_secret(), "two");
}

#[tokio::test]
async fn bedrock_aws_settings_secret_safe_failure() {
    let storage = cache(Arc::new(AtomicU32::new(0)));
    let params = BTreeMap::from([(
        "secret".to_owned(),
        Parameter::Text(SecretString::from("secret-fixture".to_owned())),
    )]);
    let error = storage
        .get_or_fetch(&params, || async { Err(SourceError::MissingProvider) })
        .await
        .unwrap_err();
    assert!(
        !format!("{error:?} {error} {params:?} {:?}", credentials("identity"))
            .contains("secret-fixture")
    );
    let retry = storage
        .get_or_fetch(&params, || async { Ok(credentials("retry")) })
        .await
        .unwrap();
    assert_eq!(retry.access_key_id.expose_secret(), "retry");
}

#[tokio::test]
async fn concurrent_misses_share_fetch() {
    let storage = cache(Arc::new(AtomicU32::new(0)));
    let calls = AtomicU32::new(0);
    let params = BTreeMap::new();
    let fetch = || async {
        calls.fetch_add(1, Ordering::SeqCst);
        yield_now().await;
        Ok(credentials("shared"))
    };
    let (first, second) = tokio::join!(
        storage.get_or_fetch(&params, fetch),
        storage.get_or_fetch(&params, fetch)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(first.unwrap().access_key_id.expose_secret(), "shared");
    assert_eq!(second.unwrap().access_key_id.expose_secret(), "shared");
}

#[tokio::test]
async fn explicit_ttl_expires_at_the_configured_boundary() {
    let clock = Arc::new(AtomicU32::new(0));
    let storage = cache(Arc::clone(&clock));
    let mut params = BTreeMap::new();
    params.insert(
        "source".to_owned(),
        Parameter::Text(SecretString::from("static".to_owned())),
    );
    let mut temporary = credentials("static");
    temporary.cache_ttl = Some(Duration::from_mins(59));
    storage
        .get_or_fetch(&params, || future::ready(Ok(temporary)))
        .await
        .unwrap();
    clock.store(3540, Ordering::SeqCst);
    storage
        .get_or_fetch(&params, || async { Err(SourceError::MissingProvider) })
        .await
        .unwrap();
    clock.store(3541, Ordering::SeqCst);
    drop(
        storage
            .get_or_fetch(&params, || async { Err(SourceError::MissingProvider) })
            .await
            .unwrap_err(),
    );
}
