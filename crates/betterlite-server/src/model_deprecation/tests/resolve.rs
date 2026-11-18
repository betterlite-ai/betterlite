use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;
use serde_json::Number;

use crate::model_deprecation::dates::parse_json_date;
use crate::model_deprecation::resolve::{Deployment, resolve_deprecation};

use super::support::{TestCatalog, calendar_date, object, text};

fn view<'deployment>(
    name: Option<&'deployment str>,
    params: &'deployment JsonObject,
    info: &'deployment JsonObject,
) -> Deployment<'deployment> {
    Deployment {
        model_name: name,
        model_info: Some(info),
        litellm_params: Some(params),
        override_date: parse_json_date(info.get("deprecation_date")),
    }
}

#[test]
fn deployment_date_override() {
    let params = object(vec![("model", text("openai/raw-model"))]);
    let info = object(vec![
        ("deprecation_date", text("2026-03-01")),
        ("litellm_provider", text("override-provider")),
    ]);
    let catalog = TestCatalog::new(vec![(
        "openai/raw-model",
        "2026-04-01",
        Some("catalog-provider"),
    )]);
    let resolved = resolve_deprecation(&view(Some("alpha"), &params, &info), &catalog)
        .expect("the override resolves");
    assert_eq!(resolved.deprecation_date, calendar_date("2026-03-01"));
    assert_eq!(resolved.litellm_model.as_deref(), Some("openai/raw-model"));
    assert_eq!(
        resolved.litellm_provider.as_deref(),
        Some("override-provider")
    );
}

#[test]
fn deployment_date_override_takes_the_provider_only_from_model_info() {
    let params = object(vec![("model", text("openai/raw-model"))]);
    let info = object(vec![("deprecation_date", text("2026-03-01"))]);
    let catalog = TestCatalog::new(vec![(
        "openai/raw-model",
        "2026-04-01",
        Some("catalog-provider"),
    )]);
    let resolved = resolve_deprecation(&view(Some("alpha"), &params, &info), &catalog)
        .expect("the override resolves");
    assert_eq!(resolved.litellm_provider, None);
}

#[test]
fn deployment_date_override_keeps_only_text_models() {
    let params = object(vec![("model", Json::Number(Number::from(7)))]);
    let info = object(vec![("deprecation_date", text("2026-03-01"))]);
    let catalog = TestCatalog::new(Vec::new());
    let resolved = resolve_deprecation(&view(Some("alpha"), &params, &info), &catalog)
        .expect("the override resolves");
    assert_eq!(resolved.litellm_model, None);
}

#[test]
fn cost_map_date() {
    let catalog = TestCatalog::new(vec![
        ("azure/base-model", "2026-05-20", Some("azure")),
        ("openai/raw-model", "2026-05-10", Some("openai")),
        ("suffix-model", "2026-05-01", None),
    ]);
    let params = object(vec![("model", text("openai/raw-model"))]);
    let base = object(vec![("base_model", text("azure/base-model"))]);
    let resolved = resolve_deprecation(&view(Some("alpha"), &params, &base), &catalog)
        .expect("the base model resolves");
    assert_eq!(resolved.litellm_model.as_deref(), Some("azure/base-model"));
    assert_eq!(resolved.litellm_provider.as_deref(), Some("azure"));

    let empty_info = JsonObject::new();
    let raw = resolve_deprecation(&view(Some("alpha"), &params, &empty_info), &catalog)
        .expect("the raw model resolves");
    assert_eq!(raw.litellm_model.as_deref(), Some("openai/raw-model"));
    assert_eq!(raw.litellm_provider.as_deref(), Some("openai"));

    let suffix_params = object(vec![("model", text("other/suffix-model"))]);
    let suffix = resolve_deprecation(&view(Some("alpha"), &suffix_params, &empty_info), &catalog)
        .expect("the suffix resolves");
    assert_eq!(suffix.litellm_model.as_deref(), Some("suffix-model"));
    assert_eq!(suffix.litellm_provider, None);
}

#[test]
fn base_model_before_raw() {
    let catalog = TestCatalog::new(vec![("openai/raw-model", "2026-05-10", Some("openai"))]);
    let params = object(vec![("model", text("openai/raw-model"))]);
    let base = object(vec![("base_model", text("missing/base"))]);
    let resolved = resolve_deprecation(&view(Some("alpha"), &params, &base), &catalog)
        .expect("the raw model resolves");
    assert_eq!(resolved.litellm_model.as_deref(), Some("openai/raw-model"));
}

#[test]
fn base_model_must_be_text() {
    let catalog = TestCatalog::new(vec![("openai/raw-model", "2026-05-10", Some("openai"))]);
    let params = object(vec![("model", text("openai/raw-model"))]);
    let base = object(vec![("base_model", Json::Bool(true))]);
    let resolved = resolve_deprecation(&view(Some("alpha"), &params, &base), &catalog)
        .expect("the raw model resolves");
    assert_eq!(resolved.litellm_model.as_deref(), Some("openai/raw-model"));
}

#[test]
fn first_slash_fallback() {
    let catalog = TestCatalog::new(vec![("gpt-4o/x", "2026-05-01", None)]);
    let params = object(vec![("model", text("openai/gpt-4o/x"))]);
    let resolved = resolve_deprecation(&view(Some("alpha"), &params, &JsonObject::new()), &catalog)
        .expect("the suffix resolves");
    assert_eq!(resolved.litellm_model.as_deref(), Some("gpt-4o/x"));
}

#[rstest]
#[case::not_a_date(text("not a date"))]
#[case::impossible_day(text("2026-02-30"))]
#[case::datetime_text(text("2026-04-01T00:00:00Z"))]
#[case::null(Json::Null)]
#[case::number(Json::Number(Number::from(20_260_401)))]
fn invalid_override_falls_through(#[case] invalid: Json) {
    let catalog = TestCatalog::new(vec![("openai/raw-model", "2026-04-01", Some("openai"))]);
    let params = object(vec![("model", text("openai/raw-model"))]);
    let info = object(vec![
        ("deprecation_date", invalid),
        ("litellm_provider", text("ignored")),
    ]);
    let resolved = resolve_deprecation(&view(Some("alpha"), &params, &info), &catalog)
        .expect("the catalog date resolves");
    assert_eq!(resolved.deprecation_date, calendar_date("2026-04-01"));
    assert_eq!(resolved.litellm_provider.as_deref(), Some("openai"));
}

#[test]
fn non_mapping_metadata() {
    let catalog = TestCatalog::new(vec![("openai/raw-model", "2026-04-01", None)]);
    let deployment = Deployment {
        model_name: Some("alpha"),
        model_info: None,
        litellm_params: None,
        override_date: None,
    };
    assert!(resolve_deprecation(&deployment, &catalog).is_none());

    let empty = JsonObject::new();
    assert!(resolve_deprecation(&view(Some("alpha"), &empty, &empty), &catalog).is_none());
}

#[test]
fn lookup_keys_must_be_non_empty() {
    let catalog = TestCatalog::new(vec![("", "2026-04-01", None)]);
    let params = object(vec![("model", text(""))]);
    let base = object(vec![("base_model", text(""))]);
    assert!(
        resolve_deprecation(&view(Some("alpha"), &params, &base), &catalog).is_none(),
        "an empty base_model, model and suffix never reach the catalog"
    );
}
