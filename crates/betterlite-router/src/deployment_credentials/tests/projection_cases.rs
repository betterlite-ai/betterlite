use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::deployment_credentials::error::CredentialError;
use crate::deployment_credentials::params::CredentialParams;
use crate::deployment_credentials::resolve::{
    deployment_credentials, deployment_credentials_with_provider,
};
use crate::deployment_identity::spec::DeploymentSpec;

use super::support::{captured, catalog_of, pairs, snapshot, spec, text};

fn named(params: &[(&str, Json)]) -> DeploymentSpec {
    spec("model-group", "d1", params, &[])
}

fn field<'params>(params: &'params CredentialParams, name: &str) -> Option<&'params str> {
    match params.fields.get(name) {
        Some(Json::Text(value)) => Some(value),
        _ => None,
    }
}

fn rich(
    catalog: &DeploymentCatalog,
    credentials: &CredentialSnapshot,
) -> (Option<CredentialParams>, Vec<String>) {
    captured(|| deployment_credentials_with_provider(catalog, "d1", None, credentials).unwrap())
}

#[test]
fn simple_lookup_no_named_resolution() {
    let catalog = catalog_of(vec![named(&[
        ("model", text("openai/gpt-4o")),
        ("api_key", text("sk-deployment")),
        ("litellm_credential_name", text("creds")),
    ])]);
    let (result, warnings) = captured(|| deployment_credentials(&catalog, "d1"));
    let params = result.unwrap().unwrap();
    assert_eq!(field(&params, "api_key"), Some("sk-deployment"));
    assert_eq!(params.fields.get("litellm_credential_name"), None);
    assert_eq!(params.fields.get("model"), None);
    assert_eq!(params.fields.get("custom_llm_provider"), None);
    assert!(warnings.is_empty());
}

#[test]
fn null_fields_omitted() {
    let catalog = catalog_of(vec![named(&[
        ("model", text("openai/gpt-4o")),
        ("api_key", Json::Null),
        ("api_base", text("https://base.test")),
        ("vertex_credentials", Json::Null),
        ("aws_session_tags", Json::Null),
    ])]);
    let params = deployment_credentials(&catalog, "d1").unwrap().unwrap();
    assert_eq!(
        params.fields,
        pairs(&[("api_base", text("https://base.test"))])
    );
}

#[test]
fn named_overlay_without_mutation() {
    let catalog = catalog_of(vec![named(&[
        ("model", text("openai/gpt-4o")),
        ("api_key", text("sk-deployment")),
        ("litellm_credential_name", text("creds")),
    ])]);
    let credentials = snapshot(&[(
        "creds",
        pairs(&[
            ("api_key", text("sk-record")),
            ("api_base", text("https://record.test")),
            ("litellm_credential_name", text("inner")),
            ("model", text("record/model")),
            ("custom_llm_provider", text("record-provider")),
        ]),
    )]);
    let before_deployment = catalog.models[0].clone();
    let before_credentials = credentials.clone();
    let (result, _warnings) = rich(&catalog, &credentials);
    let params = result.unwrap();
    assert_eq!(field(&params, "api_key"), Some("sk-record"));
    assert_eq!(field(&params, "api_base"), Some("https://record.test"));
    assert_eq!(params.fields.get("litellm_credential_name"), None);
    assert_eq!(catalog.models[0], before_deployment);
    assert_eq!(credentials, before_credentials);
}

#[test]
fn empty_name_still_lookup() {
    let catalog = catalog_of(vec![named(&[
        ("model", text("openai/gpt-4o")),
        ("api_key", text("sk-deployment")),
        ("litellm_credential_name", text("")),
    ])]);
    let credentials = snapshot(&[("", pairs(&[("api_key", text("sk-empty"))]))]);
    let (result, warnings) = rich(&catalog, &credentials);
    assert_eq!(field(&result.unwrap(), "api_key"), Some("sk-empty"));
    assert!(warnings.is_empty());
    let (result, warnings) = rich(&catalog, &snapshot(&[]));
    assert_eq!(field(&result.unwrap(), "api_key"), Some("sk-deployment"));
    assert_eq!(
        warnings,
        vec!["Credential '' not found in credential_list".to_owned()]
    );
}

#[test]
fn missing_name_warns_and_returns_base() {
    let catalog = catalog_of(vec![named(&[
        ("model", text("openai/gpt-4o")),
        ("api_key", text("sk-deployment")),
        ("litellm_credential_name", text("absent")),
    ])]);
    let (result, warnings) = rich(&catalog, &snapshot(&[]));
    assert_eq!(field(&result.unwrap(), "api_key"), Some("sk-deployment"));
    assert_eq!(
        warnings,
        vec!["Credential 'absent' not found in credential_list".to_owned()]
    );
    // a stored record with no values is an empty answer too, so it warns the same way
    let empty_record = snapshot(&[("absent", pairs(&[]))]);
    let (result, warnings) = rich(&catalog, &empty_record);
    assert_eq!(field(&result.unwrap(), "api_key"), Some("sk-deployment"));
    assert_eq!(
        warnings,
        vec!["Credential 'absent' not found in credential_list".to_owned()]
    );
}

#[test]
fn deployment_model_restored() {
    let catalog = catalog_of(vec![named(&[
        ("model", text("openai/gpt-4o")),
        ("litellm_credential_name", text("creds")),
    ])]);
    let credentials = snapshot(&[("creds", pairs(&[("model", text("record/model"))]))]);
    let (result, _warnings) = rich(&catalog, &credentials);
    assert_eq!(field(&result.unwrap(), "model"), Some("openai/gpt-4o"));
}

#[test]
fn explicit_provider_restored() {
    let catalog = catalog_of(vec![named(&[
        ("model", text("openai/gpt-4o")),
        ("custom_llm_provider", text("azure")),
        ("litellm_credential_name", text("creds")),
    ])]);
    let credentials = snapshot(&[(
        "creds",
        pairs(&[("custom_llm_provider", text("record-provider"))]),
    )]);
    let (result, _warnings) = rich(&catalog, &credentials);
    assert_eq!(
        field(&result.unwrap(), "custom_llm_provider"),
        Some("azure")
    );
}

#[test]
fn wrong_types_name_the_field() {
    let list_text = catalog_of(vec![named(&[
        ("model", text("openai/gpt-4o")),
        ("aws_session_tags", text("team")),
    ])]);
    assert!(matches!(
        deployment_credentials(&list_text, "d1").unwrap_err(),
        CredentialError::FieldType {
            field,
            expected: "a list"
        } if field == "aws_session_tags"
    ));
    let text_object = catalog_of(vec![named(&[
        ("model", text("openai/gpt-4o")),
        ("api_key", Json::Object(JsonObject::new())),
    ])]);
    assert!(matches!(
        deployment_credentials(&text_object, "d1").unwrap_err(),
        CredentialError::FieldType {
            field,
            expected: "a string or a list"
        } if field == "api_key"
    ));
    let text_list = catalog_of(vec![named(&[
        ("model", text("openai/gpt-4o")),
        ("api_base", Json::List(Vec::new())),
    ])]);
    assert!(matches!(
        deployment_credentials(&text_list, "d1").unwrap_err(),
        CredentialError::FieldType {
            field,
            expected: "a string"
        } if field == "api_base"
    ));
    let text_number = catalog_of(vec![named(&[
        ("model", text("openai/gpt-4o")),
        ("api_version", Json::Number(serde_json::Number::from(1_i64))),
    ])]);
    assert!(matches!(
        deployment_credentials(&text_number, "d1").unwrap_err(),
        CredentialError::FieldType {
            field,
            expected: "a string, an object or a list"
        } if field == "api_version"
    ));
}

#[test]
fn declared_types_accepted() {
    let vertex = Json::Object(pairs(&[("type", text("service_account"))]));
    let tags = Json::List(vec![Json::Object(pairs(&[
        ("Key", text("team")),
        ("Value", text("alpha")),
    ]))]);
    let catalog = catalog_of(vec![named(&[
        ("model", text("openai/gpt-4o")),
        ("vertex_credentials", vertex.clone()),
        ("aws_session_tags", tags.clone()),
        ("bedrock_tags", Json::List(vec![text("environment")])),
    ])]);
    let params = deployment_credentials(&catalog, "d1").unwrap().unwrap();
    assert_eq!(params.fields.get("vertex_credentials"), Some(&vertex));
    assert_eq!(params.fields.get("aws_session_tags"), Some(&tags));
}

#[test]
fn slash_provider_prefix() {
    let catalog = catalog_of(vec![named(&[("model", text("azure/gpt-4o"))])]);
    let (result, _warnings) = rich(&catalog, &snapshot(&[]));
    assert_eq!(
        field(&result.unwrap(), "custom_llm_provider"),
        Some("azure")
    );
}

#[test]
fn unprefixed_provider_openai() {
    let catalog = catalog_of(vec![named(&[("model", text("gpt-4o"))])]);
    let (result, _warnings) = rich(&catalog, &snapshot(&[]));
    assert_eq!(
        field(&result.unwrap(), "custom_llm_provider"),
        Some("openai")
    );
}

#[test]
fn credential_values_never_logged() {
    let catalog = catalog_of(vec![named(&[
        ("model", text("openai/gpt-4o")),
        ("api_key", text("sk-top-secret")),
        ("litellm_credential_name", text("absent")),
    ])]);
    let (result, warnings) = rich(&catalog, &snapshot(&[]));
    let params = result.unwrap();
    assert_eq!(
        warnings,
        vec!["Credential 'absent' not found in credential_list".to_owned()]
    );
    let debug = format!("{params:?}");
    assert!(debug.contains("api_key"));
    assert!(!debug.contains("sk-top-secret"));
    let secret = format!("{:?}", params.secret("api_key").unwrap());
    assert!(!secret.contains("sk-top-secret"));
    assert!(warnings.iter().all(|line| !line.contains("sk-top-secret")));
}
