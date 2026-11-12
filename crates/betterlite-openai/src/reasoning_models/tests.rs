#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::future::{Future, ready};
use std::sync::Arc;

use betterlite_models::model_capability_predicates::CapabilityRequest;
use rstest::rstest;
use serde_json::Value;
use tokio::runtime::Builder;
use tracing::Level;
use tracing::subscriber::with_default;

use super::gpt_5::Mapping as GptMapping;
use super::o_series::Mapping as OSeriesMapping;
use super::{
    DropParams, ProviderResolution, ReasoningCapabilities, ReasoningError, ResolvedModel, gpt_5,
    o_series,
};
use crate::openai_base_config::{CatalogCapabilities, Object};

mod o_series_cases;
mod scenarios;

pub(super) struct Fixture {
    pub(super) catalog: CatalogCapabilities,
    entries: BTreeMap<String, Value>,
}

impl Fixture {
    pub(super) fn new(models: &[&str], entries: &str) -> Self {
        Self {
            catalog: CatalogCapabilities {
                chat_models: models.iter().map(|model| (*model).to_owned()).collect(),
                text_models: BTreeSet::new(),
            },
            entries: serde_json::from_str(entries).expect("fixture entries"),
        }
    }

    fn field(&self, request: &CapabilityRequest<'_>, key: &str) -> Option<&Value> {
        let entry = match request.custom_llm_provider {
            None => {
                let known = self.catalog.chat_models.contains(request.model)
                    || self.catalog.text_models.contains(request.model);
                known.then(|| self.entries.get(request.model)).flatten()
            }
            Some(_) => self.entries.get(request.model),
        };
        entry?.get(key)
    }
}

impl ReasoningCapabilities for Fixture {
    fn supports_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(self.field(request, key).and_then(Value::as_bool) == Some(true))
    }

    fn declared_value_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        ready(
            self.field(request, key)
                .and_then(Value::as_str)
                .map(str::to_owned),
        )
    }

    fn is_explicitly_disabled_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(self.field(request, key).and_then(Value::as_bool) == Some(false))
    }

    fn supports_response_schema(
        &mut self,
        request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        ready(
            self.field(request, "supports_response_schema")
                .and_then(Value::as_bool)
                == Some(true),
        )
    }

    fn catalogue_declares_default(&mut self) -> bool {
        let entries: Vec<String> = self.entries.values().map(Value::to_string).collect();
        super::catalogue_declares_default(entries.iter().map(String::as_str))
    }
}

pub(super) struct Resolver(pub(super) Option<ResolvedModel>);

impl ProviderResolution for Resolver {
    fn resolve_provider(&mut self, _model: &str) -> Option<ResolvedModel> {
        self.0.clone()
    }
}

pub(super) fn request(source: &str) -> Object {
    let value = serde_json::from_str(source).expect("request json");
    object(value)
}

pub(super) fn object(value: Value) -> Object {
    match value {
        Value::Object(fields) => fields,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            panic!("expected an object")
        }
    }
}

pub(super) fn no_drop() -> DropParams {
    DropParams {
        global: false,
        request: false,
    }
}

pub(super) fn request_drop() -> DropParams {
    DropParams {
        global: false,
        request: true,
    }
}

pub(super) fn global_drop() -> DropParams {
    DropParams {
        global: true,
        request: false,
    }
}

pub(super) fn resolved_o3() -> ResolvedModel {
    ResolvedModel {
        model: String::from("o3"),
        provider: String::from("openai"),
    }
}

pub(super) async fn gpt5_supported(fixture: &mut Fixture, model: &str) -> Vec<&'static str> {
    let catalog = fixture.catalog.clone();
    gpt_5::supported_params(fixture, model, &catalog).await
}

pub(super) enum Family {
    Gpt,
    OSeries,
}

pub(super) struct Requests<'request> {
    pub(super) non_default: &'request mut Object,
    pub(super) optional: &'request mut Object,
    pub(super) drop_params: DropParams,
}

pub(super) async fn map_probe(
    fixture: &mut Fixture,
    resolver: &mut Resolver,
    model: &str,
    requests: Requests<'_>,
    family: Family,
) -> Result<(), ReasoningError> {
    let catalog = fixture.catalog.clone();
    let Requests {
        non_default,
        optional,
        drop_params,
    } = requests;
    match family {
        Family::Gpt => {
            gpt_5::map_params(
                fixture,
                &mut GptMapping {
                    model,
                    non_default_params: non_default,
                    optional_params: optional,
                    drop_params,
                    catalog: &catalog,
                },
            )
            .await
        }
        Family::OSeries => {
            o_series::map_params(
                fixture,
                resolver,
                &mut OSeriesMapping {
                    model,
                    non_default_params: non_default,
                    optional_params: optional,
                    drop_params,
                    catalog: &catalog,
                },
            )
            .await
        }
    }
}

pub(super) async fn map_gpt5(
    fixture: &mut Fixture,
    model: &str,
    non_default_params: &mut Object,
    optional_params: &mut Object,
    drop_params: DropParams,
) -> Result<(), ReasoningError> {
    let requests = Requests {
        non_default: non_default_params,
        optional: optional_params,
        drop_params,
    };
    map_probe(fixture, &mut Resolver(None), model, requests, Family::Gpt).await
}

enum Case {
    Gpt5SearchAllowlist,
    Gpt5ChatExclusion,
    Gpt6FamilySelection,
    DictEffortNormalization,
    XhighOptIn,
    LowMinimalOptOut,
    CatalogBeforeDefaultKey,
    TemperatureWhileThinking,
    SamplingWithNone,
    O1DeveloperRoles,
    AudioTokenParameter,
    Gpt5RegularAllowlist,
    VersionBoundaries,
    OSeriesCapabilityFallback,
    OSeriesTemperatureLimit,
}

#[rstest]
#[case::gpt5_search_allowlist(Case::Gpt5SearchAllowlist)]
#[case::gpt5_chat_exclusion(Case::Gpt5ChatExclusion)]
#[case::gpt6_family_selection(Case::Gpt6FamilySelection)]
#[case::dict_effort_normalization(Case::DictEffortNormalization)]
#[case::xhigh_opt_in(Case::XhighOptIn)]
#[case::low_minimal_opt_out(Case::LowMinimalOptOut)]
#[case::catalog_before_default_key(Case::CatalogBeforeDefaultKey)]
#[case::temperature_while_thinking(Case::TemperatureWhileThinking)]
#[case::sampling_with_none(Case::SamplingWithNone)]
#[case::o1_developer_roles(Case::O1DeveloperRoles)]
#[case::audio_token_parameter(Case::AudioTokenParameter)]
#[case::gpt5_regular_allowlist(Case::Gpt5RegularAllowlist)]
#[case::version_boundaries(Case::VersionBoundaries)]
#[case::o_series_capability_fallback(Case::OSeriesCapabilityFallback)]
#[case::o_series_temperature_limit(Case::OSeriesTemperatureLimit)]
#[tokio::test]
async fn reasoning_models_cases(#[case] case: Case) {
    match case {
        Case::Gpt5SearchAllowlist => scenarios::gpt5_search_allowlist().await,
        Case::Gpt5ChatExclusion => scenarios::gpt5_chat_exclusion(),
        Case::Gpt6FamilySelection => scenarios::gpt6_family_selection().await,
        Case::DictEffortNormalization => scenarios::dict_effort_normalization().await,
        Case::XhighOptIn => scenarios::xhigh_opt_in().await,
        Case::LowMinimalOptOut => scenarios::low_minimal_opt_out().await,
        Case::CatalogBeforeDefaultKey => scenarios::catalog_before_default_key().await,
        Case::TemperatureWhileThinking => scenarios::temperature_while_thinking().await,
        Case::SamplingWithNone => scenarios::sampling_with_none().await,
        Case::O1DeveloperRoles => scenarios::o1_developer_roles().await,
        Case::AudioTokenParameter => scenarios::audio_token_parameter(),
        Case::Gpt5RegularAllowlist => scenarios::gpt5_regular_allowlist().await,
        Case::VersionBoundaries => scenarios::version_boundaries(),
        Case::OSeriesCapabilityFallback => o_series_cases::o_series_capability_fallback().await,
        Case::OSeriesTemperatureLimit => o_series_cases::o_series_temperature_limit().await,
    }
}

#[test]
fn o_series_provider_fallback_is_logged_at_debug() {
    let log = tempfile::NamedTempFile::new().expect("log file");
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(Level::DEBUG)
        .with_writer(Arc::new(log.reopen().expect("log handle")))
        .finish();
    with_default(subscriber, || {
        let mut fixture = Fixture::new(
            &[],
            r#"{"fixture-model": {"litellm_provider": "openai", "supports_function_calling": true}}"#,
        );
        let catalog = fixture.catalog.clone();
        let runtime = Builder::new_current_thread().build().expect("runtime");
        let supported = runtime.block_on(o_series::supported_params(
            &mut fixture,
            &mut Resolver(None),
            "fixture-model",
            &catalog,
        ));
        assert!(supported.contains(&"tools"));
    });
    let text = fs::read_to_string(log.path()).expect("log text");
    assert!(text.contains("o-series"), "{text}");
    assert!(text.contains("openai"), "{text}");
    assert!(text.contains("DEBUG"), "{text}");
}
