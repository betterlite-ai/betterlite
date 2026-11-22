#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::time::Duration;

use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::{Extra, Json};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use crate::credentials::LiteLlmGlobals;
use crate::decisions::call::azure::AzureAiConfig;
use crate::decisions::call::error::PrepareError;
use crate::decisions::call::fields::{
    OpenAiDecisionRequestFields, RequestFields, SystemOneRequestFields,
};
use crate::decisions::call::{DecisionsCall, PrepareCall, prepare_call};
use crate::decisions::openai::request::wire::RequestWire as OpenAiRequestWire;
use crate::decisions::transformation::wire::{
    ProviderRequestWire, RequestWire as SystemOneRequestWire,
};
use crate::openai_base_config::Headers;

pub(super) struct Fixture {
    pub(super) catalog: ModelCatalog,
    pub(super) known: BTreeSet<String>,
    pub(super) custom: BTreeSet<String>,
    pub(super) environment: BTreeMap<String, String>,
    pub(super) logging: Mutex<LoggingState>,
}

impl Default for Fixture {
    fn default() -> Self {
        Self {
            catalog: ModelCatalog::default(),
            known: BTreeSet::new(),
            custom: BTreeSet::new(),
            environment: BTreeMap::new(),
            logging: Mutex::new(logging_state()),
        }
    }
}

impl Fixture {
    pub(super) fn with_environment(environment: BTreeMap<String, String>) -> Self {
        Self {
            catalog: ModelCatalog::default(),
            known: BTreeSet::new(),
            custom: BTreeSet::new(),
            environment,
            logging: Mutex::new(logging_state()),
        }
    }

    pub(super) fn context(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.known,
            custom_providers: &self.custom,
            environment: &self.environment,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }

    pub(super) fn prepare<'logging>(
        &self,
        call: CallBuilder<'_>,
        logging_obj: Option<&'logging Mutex<LoggingState>>,
    ) -> Result<DecisionsCall<'logging>, PrepareError> {
        let context = self.context();
        prepare_call(PrepareCall {
            model: call.model,
            fields: call.fields,
            safety_identifier: call.safety_identifier,
            api_key: call.api_key,
            api_base: call.api_base,
            timeout: call.timeout,
            custom_llm_provider: call.custom_llm_provider,
            extra_headers: call.extra_headers,
            kwargs: call.kwargs,
            global_drop_params: call.global_drop_params,
            globals: call.globals,
            azure_ai: call.azure_ai,
            resolution: &context,
            logging_obj,
        })
    }

    pub(super) fn debug_prepare_call(&self, call: CallBuilder<'_>) -> String {
        let context = self.context();
        let prepare = PrepareCall {
            model: call.model,
            fields: call.fields,
            safety_identifier: call.safety_identifier,
            api_key: call.api_key,
            api_base: call.api_base,
            timeout: call.timeout,
            custom_llm_provider: call.custom_llm_provider,
            extra_headers: call.extra_headers,
            kwargs: call.kwargs,
            global_drop_params: call.global_drop_params,
            globals: call.globals,
            azure_ai: call.azure_ai,
            resolution: &context,
            logging_obj: None,
        };
        format!("{prepare:?}")
    }
}

pub(super) struct CallBuilder<'call> {
    pub(super) model: &'call str,
    pub(super) fields: RequestFields,
    pub(super) safety_identifier: Option<&'call str>,
    pub(super) api_key: Option<&'call SecretString>,
    pub(super) api_base: Option<&'call str>,
    pub(super) timeout: Option<Duration>,
    pub(super) custom_llm_provider: Option<&'call str>,
    pub(super) extra_headers: Option<&'call Headers>,
    pub(super) kwargs: Extra,
    pub(super) global_drop_params: bool,
    pub(super) globals: LiteLlmGlobals,
    pub(super) azure_ai: AzureAiConfig,
}

impl<'call> CallBuilder<'call> {
    pub(super) fn new(model: &'call str, fields: RequestFields) -> Self {
        Self {
            model,
            fields,
            safety_identifier: None,
            api_key: None,
            api_base: None,
            timeout: None,
            custom_llm_provider: None,
            extra_headers: None,
            kwargs: Extra::new(),
            global_drop_params: false,
            globals: LiteLlmGlobals {
                api_base: None,
                api_key: None,
                openai_key: None,
                organization: None,
            },
            azure_ai: azure_ai(),
        }
    }
}

pub(super) fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

pub(super) fn exposed(key: Option<&SecretString>) -> Option<&str> {
    match key {
        Some(secret) => Some(secret.expose_secret()),
        None => None,
    }
}

pub(super) fn globals(api_base: &str, api_key: SecretString) -> LiteLlmGlobals {
    LiteLlmGlobals {
        api_base: Some(api_base.to_owned()),
        api_key: Some(api_key),
        openai_key: None,
        organization: None,
    }
}

pub(super) fn environment(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn azure_ai() -> AzureAiConfig {
    AzureAiConfig::new(Box::new(|api_base: &str, _model: &str| {
        let base = api_base.trim_end_matches('/');
        format!("{base}/providers/microsoft/v1/systemone")
    }))
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(super) fn question_map(entries: &[(&str, Json)]) -> IndexMap<String, Json> {
    entries
        .iter()
        .map(|(name, question)| ((*name).to_owned(), question.clone()))
        .collect()
}

pub(super) fn noul_question() -> Json {
    json(r#"{"type":"noul","instructions":"Is it true?"}"#)
}

pub(super) fn predicate_question() -> Json {
    json(r#"{"type":"predicate","name":"first","instructions":"Is it true?"}"#)
}

fn openai_fields(input: Json, questions: Vec<Json>) -> RequestFields {
    RequestFields::OpenAi(OpenAiDecisionRequestFields {
        input: Some(input),
        questions: Some(questions),
    })
}

fn systemone_fields(state: &str, question: Json) -> RequestFields {
    RequestFields::SystemOne(SystemOneRequestFields {
        state: Some(Json::Text(state.to_owned())),
        questions: Some(question_map(&[("first", question)])),
    })
}

pub(super) fn good_systemone_fields(state: &str) -> RequestFields {
    systemone_fields(state, noul_question())
}

pub(super) fn openai_predicate_fields(input: &str) -> RequestFields {
    openai_fields(Json::Text(input.to_owned()), vec![predicate_question()])
}

pub(super) fn openai_image_fields() -> RequestFields {
    openai_fields(
        json(
            r#"[{"role":"user","type":"message","content":[{"type":"input_text","text":"look"},{"type":"input_image","image_url":"https://example.com/a.png"}]}]"#,
        ),
        vec![predicate_question()],
    )
}

pub(super) fn openai_empty_name_fields() -> RequestFields {
    openai_fields(
        Json::Text("state".to_owned()),
        vec![json(
            r#"{"type":"predicate","name":"","instructions":"Is it true?"}"#,
        )],
    )
}

pub(super) fn systemone_single_choice_fields() -> RequestFields {
    systemone_fields(
        "state",
        json(r#"{"type":"choice","instructions":"Pick one","criteria":{"only":null}}"#),
    )
}

pub(super) fn systemone_wire(body: &ProviderRequestWire) -> &SystemOneRequestWire {
    match body {
        ProviderRequestWire::SystemOne(wire) => wire,
        ProviderRequestWire::OpenAi(_wire) => panic!("expected a SystemOne request body"),
    }
}

pub(super) fn openai_wire(body: &ProviderRequestWire) -> &OpenAiRequestWire {
    match body {
        ProviderRequestWire::OpenAi(wire) => wire,
        ProviderRequestWire::SystemOne(_wire) => panic!("expected an OpenAI request body"),
    }
}

pub(super) fn logging_state() -> LoggingState {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"decision","call_type":"decisions","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1"}"#,
    )
    .unwrap();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || "generated".to_owned(),
        Duration::default,
    )
    .unwrap()
}
