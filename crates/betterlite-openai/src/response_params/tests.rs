#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Arc;

use betterlite_models::model_capability_predicates::CapabilityRequest;
use rstest::rstest;
use serde_json::Value;
use tokio::runtime::{Builder, Runtime};

use super::bundled::BundledReasoningModels;
use super::capabilities::{ModelInfo, ResponsesCapabilities};
use super::error::ResponseParamsError;
use super::map_openai_params;
use super::params::{ResponsesOptions, ResponsesParams};
use crate::reasoning_models::{DropParams, ReasoningCapabilities};

mod corpus;
mod scenarios;

#[derive(Clone, Copy)]
enum Scenario {
    MinimumOutputTokens,
    ReasoningUnsupportedDropOrError,
    Gpt5TemperatureDefaultEffort,
    TopPWithReasoning,
    UnknownCatalogModel,
    ReasoningSummaryWithoutEffort,
    ResponsesParameterSet,
    ProviderCapabilities,
}

#[derive(Clone)]
pub(super) struct Fixture {
    entries: BTreeMap<String, Value>,
}

impl Fixture {
    fn new(entries: &Value) -> Self {
        Self {
            entries: serde_json::from_value(entries.clone()).expect("entries"),
        }
    }

    fn field(&self, model: &str, key: &str) -> Option<&Value> {
        if let Some(entry) = self.entries.get(model) {
            return entry.get(key);
        }
        let (_, name) = model.rsplit_once('/')?;
        self.entries.get(name)?.get(key)
    }
}

impl ReasoningCapabilities for Fixture {
    fn supports_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(self.field(request.model, key).and_then(Value::as_bool) == Some(true))
    }

    fn declared_value_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        ready(
            self.field(request.model, key)
                .and_then(Value::as_str)
                .map(str::to_owned),
        )
    }

    fn is_explicitly_disabled_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(self.field(request.model, key).and_then(Value::as_bool) == Some(false))
    }

    fn supports_response_schema(
        &mut self,
        request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        ready(
            self.field(request.model, "supports_response_schema")
                .and_then(Value::as_bool)
                == Some(true),
        )
    }

    fn catalogue_declares_default(&mut self) -> bool {
        self.entries
            .values()
            .any(|entry| entry.get("default_reasoning_effort").is_some())
    }
}

impl ResponsesCapabilities for Fixture {
    fn model_info(&mut self, model: &str) -> impl Future<Output = ModelInfo> + Send {
        ready(
            self.entries
                .get(model)
                .map_or(ModelInfo::Unavailable, |entry| ModelInfo::Found {
                    key: model.to_owned(),
                    supports_reasoning: entry.get("supports_reasoning").and_then(Value::as_bool),
                }),
        )
    }
}

pub(super) struct Probe {
    entries: Value,
    options: Value,
    drop_params: DropParams,
}

impl Probe {
    pub(super) fn new(entries: Value, options: Value) -> Self {
        Self {
            entries,
            options,
            drop_params: DropParams::default(),
        }
    }

    pub(super) fn dropping_request(mut self) -> Self {
        self.drop_params = DropParams {
            global: false,
            request: true,
        };
        self
    }

    pub(super) fn dropping_globally(mut self) -> Self {
        self.drop_params = DropParams {
            global: true,
            request: false,
        };
        self
    }
}

struct Prepared {
    fixture: Fixture,
    options: ResponsesOptions,
    drop_params: DropParams,
    runtime: Runtime,
}

impl Prepared {
    fn new(options: Value, fixture: Fixture, drop_params: DropParams) -> Self {
        Self {
            fixture,
            options: serde_json::from_value(options).expect("options"),
            drop_params,
            runtime: Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime"),
        }
    }
}

pub(super) fn map(
    bundled: &mut BundledReasoningModels,
    probe: Probe,
) -> Result<ResponsesParams, ResponseParamsError> {
    let Probe {
        entries,
        options,
        drop_params,
    } = probe;
    let mut prepared = Prepared::new(options, Fixture::new(&entries), drop_params);
    prepared.runtime.block_on(map_openai_params(
        &mut prepared.fixture,
        &prepared.options,
        prepared.drop_params,
        bundled,
    ))
}

pub(super) fn mapped(bundled: &mut BundledReasoningModels, probe: Probe) -> Value {
    match map(bundled, probe) {
        Ok(params) => serde_json::to_value(params).expect("params"),
        Err(error) => panic!("unexpected error: {error}"),
    }
}

pub(super) fn rejected(bundled: &mut BundledReasoningModels, probe: Probe) -> ResponseParamsError {
    match map(bundled, probe) {
        Ok(params) => panic!(
            "expected an error, mapped {}",
            serde_json::to_value(params).expect("params")
        ),
        Err(error) => {
            assert_eq!(error.status_code(), 400);
            error
        }
    }
}

pub(super) fn assert_mapped(bundled: &mut BundledReasoningModels, probe: Probe, expected: &Value) {
    let mapped = mapped(bundled, probe);
    assert_eq!(&mapped, expected);
}

#[rstest]
#[case::minimum_output_tokens(Scenario::MinimumOutputTokens)]
#[case::reasoning_unsupported_drop_or_error(Scenario::ReasoningUnsupportedDropOrError)]
#[case::gpt5_temperature_default_effort(Scenario::Gpt5TemperatureDefaultEffort)]
#[case::top_p_with_reasoning(Scenario::TopPWithReasoning)]
#[case::unknown_catalog_model(Scenario::UnknownCatalogModel)]
#[case::reasoning_summary_without_effort(Scenario::ReasoningSummaryWithoutEffort)]
#[case::responses_parameter_set(Scenario::ResponsesParameterSet)]
#[case::provider_capabilities(Scenario::ProviderCapabilities)]
#[test]
fn response_params_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::MinimumOutputTokens => scenarios::minimum_output_tokens(),
        Scenario::ReasoningUnsupportedDropOrError => {
            scenarios::reasoning_unsupported_drop_or_error();
        }
        Scenario::Gpt5TemperatureDefaultEffort => scenarios::gpt5_temperature_default_effort(),
        Scenario::TopPWithReasoning => scenarios::top_p_with_reasoning(),
        Scenario::UnknownCatalogModel => scenarios::unknown_catalog_model(),
        Scenario::ReasoningSummaryWithoutEffort => scenarios::reasoning_summary_without_effort(),
        Scenario::ResponsesParameterSet => scenarios::responses_parameter_set(),
        Scenario::ProviderCapabilities => {
            scenarios::provider_capability_flags();
        }
    }
}

#[test]
fn bundled_reasoning_models_follow_the_pinned_catalog() {
    let models = BundledReasoningModels::default().get();
    assert!(
        models.contains("gpt-5.2"),
        "the openai reasoning set is empty"
    );
    assert!(models.contains("o3-mini"));
    assert!(!models.contains("gpt-4o"));
    assert!(!models.contains("deepseek-reasoner"));
}

#[test]
fn bundled_reasoning_models_are_cached() {
    let mut bundled = BundledReasoningModels::default();
    let first = bundled.get();
    let second = bundled.get();
    assert!(Arc::ptr_eq(&first, &second));
}
