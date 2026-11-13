#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::resolution::ResolutionError;
use rstest::rstest;

use super::error::PillarError;
use super::model_provider::{CatalogResolver, ModelProviderResolver, extract_model_and_provider};

const CATALOG: &str = r#"{
    "gpt-4o": {"litellm_provider": "openai", "mode": "chat"},
    "claude-3-5-sonnet-20240620": {"litellm_provider": "anthropic"}
}"#;

fn catalog() -> ModelCatalog {
    ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject).expect("the test catalog parses")
}

struct Fixture {
    catalog: ModelCatalog,
    known: BTreeSet<String>,
    custom: BTreeSet<String>,
    environment: BTreeMap<String, String>,
    registry: JSONProviderRegistry,
}

impl Fixture {
    fn new() -> Self {
        Self {
            catalog: catalog(),
            known: BTreeSet::new(),
            custom: BTreeSet::new(),
            environment: BTreeMap::new(),
            registry: JSONProviderRegistry::default(),
        }
    }

    fn resolver(&self) -> CatalogResolver<'_> {
        CatalogResolver {
            catalog: &self.catalog,
            known_model_names: &self.known,
            custom_providers: &self.custom,
            environment: &self.environment,
            registry: &self.registry,
        }
    }
}

fn data(entries: &[(&str, GuardrailValue)]) -> BTreeMap<String, GuardrailValue> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

#[rstest]
#[case::explicit_prefix("openai/gpt-4o", "gpt-4o", "openai")]
#[case::catalog_inference("gpt-4o", "gpt-4o", "openai")]
#[case::anthropic_catalog_entry(
    "claude-3-5-sonnet-20240620",
    "claude-3-5-sonnet-20240620",
    "anthropic"
)]
fn a_known_model_is_cleaned_and_its_provider_resolved(
    #[case] model: &str,
    #[case] clean: &str,
    #[case] provider: &str,
) {
    let fixture = Fixture::new();
    let data = data(&[("model", text(model))]);
    let resolved =
        extract_model_and_provider(&data, &fixture.resolver()).expect("a model resolves");
    assert_eq!(resolved, (clean.to_owned(), provider.to_owned()));
}

#[test]
fn an_unknown_model_falls_back_to_the_model_and_no_provider() {
    let fixture = Fixture::new();
    let data = data(&[("model", text("unknown-model-xyz"))]);
    let resolved =
        extract_model_and_provider(&data, &fixture.resolver()).expect("a model resolves");
    assert_eq!(
        resolved,
        ("unknown-model-xyz".to_owned(), "unknown".to_owned())
    );
}

#[test]
fn a_failed_resolution_uses_the_declared_provider_field() {
    let fixture = Fixture::new();
    let data = data(&[
        ("model", text("unknown-model-xyz")),
        ("provider", text("acme")),
    ]);
    let resolved =
        extract_model_and_provider(&data, &fixture.resolver()).expect("a model resolves");
    assert_eq!(
        resolved,
        ("unknown-model-xyz".to_owned(), "acme".to_owned())
    );
}

struct FailingResolver;

impl ModelProviderResolver for FailingResolver {
    fn model_and_provider(
        &self,
        model: &str,
        _custom_llm_provider: Option<&str>,
        _api_base: Option<&str>,
        _api_key: Option<&str>,
    ) -> Result<(String, String), ResolutionError> {
        Err(ResolutionError::MissingProvider {
            model: model.to_owned(),
        })
    }
}

#[test]
fn a_failed_resolution_prefers_custom_llm_provider_over_the_provider_field() {
    let data = data(&[
        ("model", text("unknown-model-xyz")),
        ("custom_llm_provider", text("acme")),
        ("provider", text("fallback")),
    ]);
    let resolved = extract_model_and_provider(&data, &FailingResolver).expect("a model resolves");
    assert_eq!(
        resolved,
        ("unknown-model-xyz".to_owned(), "acme".to_owned())
    );
}

#[test]
fn a_failed_resolution_without_a_declaration_reports_unknown() {
    let data = data(&[("model", text("unknown-model-xyz"))]);
    let resolved = extract_model_and_provider(&data, &FailingResolver).expect("a model resolves");
    assert_eq!(
        resolved,
        ("unknown-model-xyz".to_owned(), "unknown".to_owned())
    );
}

#[rstest]
#[case::absent(&[])]
#[case::null(&[("model", GuardrailValue::Null)])]
#[case::empty(&[("model", GuardrailValue::Text(String::new()))])]
fn a_missing_model_is_unknown_provider(#[case] entries: &[(&str, GuardrailValue)]) {
    let fixture = Fixture::new();
    let resolved = extract_model_and_provider(&data(entries), &fixture.resolver())
        .expect("a missing model is not an error");
    assert_eq!(resolved, ("unknown".to_owned(), "unknown".to_owned()));
}

#[rstest]
#[case::model("model", GuardrailValue::SignedInteger(4))]
#[case::custom_provider("custom_llm_provider", GuardrailValue::Boolean(true))]
#[case::api_base("api_base", GuardrailValue::SignedInteger(1))]
#[case::api_key("api_key", GuardrailValue::Array(Vec::new()))]
fn a_wrongly_typed_field_names_itself(#[case] field: &'static str, #[case] value: GuardrailValue) {
    let fixture = Fixture::new();
    let entries: Vec<(&str, GuardrailValue)> =
        vec![("model", text("openai/gpt-4o")), (field, value)];
    let error = extract_model_and_provider(&data(&entries), &fixture.resolver())
        .expect_err("a wrongly typed field is rejected");
    assert!(matches!(error, PillarError::Field { field: named, .. } if named == field));
}
