#![cfg(test)]

mod override_cases;
mod region_cases;
mod route_cases;
mod signing_cases;
mod support;
mod workspace_cases;

use std::collections::BTreeMap;

use betterlite_config::sections::claude_platform_inputs::Settings as PlatformSettings;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::value::RawValue;

use crate::aws_settings::sources::Credentials;
use crate::claude_platform_inputs::endpoint::{UrlInputs, complete_url, resolve_region};
use crate::claude_platform_inputs::error::PlatformInputError;
use crate::claude_platform_inputs::signing::{PlatformSignRequest, PlatformSignature};
use crate::claude_platform_inputs::workspace::resolve_workspace_id;
use crate::credentials::error::CredentialError;
use crate::model_routes::ParamValue;
use crate::signing::target::RegionInputs;

#[derive(Debug, Clone, Copy)]
enum Scenario {
    WorkspaceAliasPrecedence,
    ExplicitRegionRequired,
    ApiBaseBeforeRegion,
    PlatformRoutePrefix,
    UnsupportedOverride,
    FilteredBodyFields,
    PlatformSigning,
}

#[rstest]
#[case::workspace_alias_precedence(Scenario::WorkspaceAliasPrecedence)]
#[case::explicit_region_required(Scenario::ExplicitRegionRequired)]
#[case::api_base_before_region(Scenario::ApiBaseBeforeRegion)]
#[case::platform_route_prefix(Scenario::PlatformRoutePrefix)]
#[case::unsupported_override(Scenario::UnsupportedOverride)]
#[case::filtered_body_fields(Scenario::FilteredBodyFields)]
#[case::platform_signing(Scenario::PlatformSigning)]
fn claude_platform_inputs_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::WorkspaceAliasPrecedence => workspace_cases::workspace_alias_precedence(),
        Scenario::ExplicitRegionRequired => region_cases::explicit_region_required(),
        Scenario::ApiBaseBeforeRegion => region_cases::api_base_before_region(),
        Scenario::PlatformRoutePrefix => route_cases::platform_route_prefix(),
        Scenario::UnsupportedOverride => override_cases::unsupported_override(),
        Scenario::FilteredBodyFields => override_cases::filtered_body_fields(),
        Scenario::PlatformSigning => signing_cases::platform_signing(),
    }
}

fn raw_optional_params(entries: &[(&str, &str)]) -> BTreeMap<String, Box<RawValue>> {
    entries
        .iter()
        .map(|(key, value)| {
            (
                (*key).to_owned(),
                RawValue::from_string(format!("\"{value}\"")).unwrap(),
            )
        })
        .collect()
}

fn two_identities() -> (PlatformSettings, PlatformSettings) {
    let first = support::settings(&[
        ("ANTHROPIC_AWS_API_KEY", "first-fixture-key"),
        ("ANTHROPIC_AWS_BASE_URL", "https://first.example"),
        ("ANTHROPIC_AWS_WORKSPACE_ID", "first-workspace"),
    ]);
    let second = support::settings(&[
        ("ANTHROPIC_AWS_API_KEY", "second-fixture-key"),
        ("ANTHROPIC_AWS_BASE_URL", "https://second.example"),
        ("ANTHROPIC_AWS_WORKSPACE_ID", "second-workspace"),
    ]);
    (first, second)
}

#[test]
fn claude_platform_inputs_credential_identity_isolation() {
    let (first, second) = two_identities();
    workspaces_stay_separate(&first, &second);
    urls_stay_separate(&first, &second);
    identity_keys_stay_separate(&first, &second);
    debug_outputs_stay_separate(&first, &second);
}

fn workspaces_stay_separate(first: &PlatformSettings, second: &PlatformSettings) {
    let empty_inputs = support::empty_inputs();
    let empty_params = support::params(&[]);
    assert_eq!(
        resolve_workspace_id(
            &empty_params,
            &empty_inputs,
            &empty_inputs,
            &first.environment
        )
        .unwrap(),
        Some("first-workspace".to_owned())
    );
    assert_eq!(
        resolve_workspace_id(
            &empty_params,
            &empty_inputs,
            &empty_inputs,
            &second.environment
        )
        .unwrap(),
        Some("second-workspace".to_owned())
    );
}

fn urls_stay_separate(first: &PlatformSettings, second: &PlatformSettings) {
    let empty_params = support::params(&[]);
    let aws = support::aws_endpoints(&[]);
    let first_url = complete_url(
        &UrlInputs {
            api_base: None,
            litellm_api_base: None,
            optional_params: &empty_params,
        },
        &first.environment,
        &aws,
    )
    .unwrap();
    let second_url = complete_url(
        &UrlInputs {
            api_base: None,
            litellm_api_base: None,
            optional_params: &empty_params,
        },
        &second.environment,
        &aws,
    )
    .unwrap();
    assert_eq!(first_url, "https://first.example/v1/messages");
    assert_eq!(second_url, "https://second.example/v1/messages");
    assert_ne!(first_url, second_url);
}

fn identity_keys_stay_separate(first: &PlatformSettings, second: &PlatformSettings) {
    let first_key = first.environment.anthropic_aws_api_key.as_ref().unwrap();
    let second_key = second.environment.anthropic_aws_api_key.as_ref().unwrap();
    assert_ne!(first_key.expose_secret(), second_key.expose_secret());
    assert_eq!(
        sign_with_identity("https://first.example", "first", first_key),
        [("x-identity".to_owned(), Some("first".to_owned()))]
    );
    assert_eq!(
        sign_with_identity("https://second.example", "second", second_key),
        [("x-identity".to_owned(), Some("second".to_owned()))]
    );
}

fn debug_outputs_stay_separate(first: &PlatformSettings, second: &PlatformSettings) {
    assert!(!format!("{:?}", first.environment).contains("second-fixture-key"));
    assert!(!format!("{:?}", second.environment).contains("first-fixture-key"));
    assert!(!format!("{first:?}").contains("second-workspace"));
    assert!(!format!("{second:?}").contains("first-workspace"));
}

fn sign_with_identity(
    api_base: &str,
    identity: &str,
    key: &SecretString,
) -> Vec<(String, Option<String>)> {
    let headers = [("x-identity".to_owned(), Some(identity.to_owned()))];
    let data = support::request_data();
    let optional_params = BTreeMap::new();
    let signature = support::sign_with(
        PlatformSignRequest {
            headers: &headers,
            optional_params: &optional_params,
            request_data: &data,
            api_base,
            api_key: None,
            environment_api_key: Some(key),
            configured_bearer: None,
            model: None,
            region_inputs: RegionInputs::default(),
            now: support::epoch(1_767_225_600),
        },
        |_settings, _region| Ok(support::credentials()),
    )
    .unwrap();
    match signature {
        PlatformSignature::Supplied { headers: supplied } => supplied,
        PlatformSignature::Signed(signed) => panic!("identity was signed at {}", signed.url),
    }
}

#[test]
fn claude_platform_inputs_secret_safe_failure() {
    let workspace_secret = "fixture-workspace-secret";
    let api_secret = "fixture-api-secret";
    let credential_secret = "fixture-credential-secret";
    let settings = support::settings(&[
        ("ANTHROPIC_AWS_WORKSPACE_ID", workspace_secret),
        ("ANTHROPIC_AWS_API_KEY", api_secret),
        ("ANTHROPIC_AWS_BASE_URL", "https://fixture.example"),
    ]);
    let secrets = [workspace_secret, api_secret, credential_secret];
    debug_outputs_are_redacted(&settings, &secrets);
    owned_errors_do_not_leak_secrets(&settings, credential_secret);
    signing_failures_do_not_leak_secrets(credential_secret);
    signed_output_is_redacted(credential_secret);
}

fn debug_outputs_are_redacted(settings: &PlatformSettings, secrets: &[&str; 3]) {
    let rendered_settings = format!("{settings:?}");
    let rendered_environment = format!("{:?}", settings.environment);
    for secret in secrets {
        assert!(!rendered_settings.contains(secret));
        assert!(!rendered_environment.contains(secret));
    }
}

fn owned_errors_do_not_leak_secrets(settings: &PlatformSettings, credential_secret: &str) {
    let aws = support::aws_endpoints(&[]);
    let invalid = resolve_region(
        &support::params(&[("aws_region_name", support::text("BAD_REGION"))]),
        &aws,
    )
    .unwrap_err();
    assert!(
        matches!(invalid, PlatformInputError::Region(_)),
        "{invalid}"
    );
    let missing = resolve_region(&support::params(&[]), &aws).unwrap_err();
    assert!(
        matches!(missing, PlatformInputError::MissingRegion),
        "{missing}"
    );
    let wrong_type = resolve_workspace_id(
        &support::params(&[(
            "workspace_id",
            ParamValue::List(vec![support::text(credential_secret)]),
        )]),
        &support::empty_inputs(),
        &support::empty_inputs(),
        &settings.environment,
    )
    .unwrap_err();
    assert!(
        matches!(wrong_type, PlatformInputError::ValueType { .. }),
        "{wrong_type}"
    );
    for error in [&invalid, &missing, &wrong_type] {
        let rendered_error = format!("{error} {error:?}");
        assert!(!rendered_error.contains(credential_secret));
        assert!(!rendered_error.contains("fixture-workspace-secret"));
        assert!(!rendered_error.contains("fixture-api-secret"));
    }
}

fn signing_failures_do_not_leak_secrets(credential_secret: &str) {
    let optional_params = raw_optional_params(&[("aws_region_name", "eu-west-1")]);
    let data = support::request_data();
    let headers = [(
        "content-type".to_owned(),
        Some("application/json".to_owned()),
    )];
    let failure = support::sign_with(
        PlatformSignRequest {
            headers: &headers,
            optional_params: &optional_params,
            request_data: &data,
            api_base: "https://aws-external-anthropic.eu-west-1.api.aws/v1/messages",
            api_key: None,
            environment_api_key: None,
            configured_bearer: None,
            model: None,
            region_inputs: RegionInputs::default(),
            now: support::epoch(1_767_225_600),
        },
        |_settings, _region| Err(CredentialError::MissingCredentials),
    )
    .unwrap_err();
    assert!(
        matches!(failure, PlatformInputError::Signing(_)),
        "{failure}"
    );
    assert!(!format!("{failure} {failure:?}").contains(credential_secret));
}

fn signed_output_is_redacted(credential_secret: &str) {
    let optional_params = raw_optional_params(&[("aws_region_name", "eu-west-1")]);
    let data = support::request_data();
    let headers = [(
        "content-type".to_owned(),
        Some("application/json".to_owned()),
    )];
    let credentials = Credentials {
        access_key_id: SecretString::from(credential_secret),
        secret_access_key: SecretString::from(credential_secret),
        session_token: None,
    };
    let signature = support::sign_with(
        PlatformSignRequest {
            headers: &headers,
            optional_params: &optional_params,
            request_data: &data,
            api_base: "https://aws-external-anthropic.eu-west-1.api.aws/v1/messages",
            api_key: None,
            environment_api_key: None,
            configured_bearer: None,
            model: None,
            region_inputs: RegionInputs::default(),
            now: support::epoch(1_767_225_600),
        },
        |_settings, _region| Ok(credentials.clone()),
    )
    .unwrap();
    assert!(!format!("{signature:?}").contains(credential_secret));
}
