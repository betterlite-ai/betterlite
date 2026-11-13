use betterlite_context::bedrock_config_selection::chat::BedrockRoute;
use betterlite_context::chat_config_selection::facts::{BedrockFacts, ModelFamilyProbe};
use betterlite_context::chat_config_selection::json_provider::{
    JsonProviderLookup, JsonProviderProjection,
};
use betterlite_context::chat_config_selection::selection::{
    ChatConfigError, ChatConfigRequest, select_chat_config,
};
use betterlite_context::chat_config_selection::table::ProviderConfigTable;
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_context::chat_config_selection::vertex::VertexModelLists;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::resolution::ResolutionError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_streaming::stream_base_assembly::provider_metadata::{
    ChatConfigSelection, ProjectionError, ProviderMetadataContext, ProviderMetadataProjection,
    ProviderResolution, apply_provider_metadata,
};
use betterlite_streaming::stream_base_assembly::records::Chunk;
use betterlite_streaming::stream_base_assembly::response::BaseResponse;
use serde_json::{Value, json};

use super::views::{fixture_response, response_view};
use super::{ProjectionFixture, ProviderScenario, ResponseSpec, SelectorFixture};

pub(super) fn assembled_json(cases: &[ProviderScenario]) -> crate::helpers::HelperResult {
    let redactor = match Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH) {
        Ok(redactor) => redactor,
        Err(error) => return Err(failure(error.to_string())),
    };
    let results: Vec<Value> = cases
        .iter()
        .map(|scenario| assembled_view(scenario, &redactor))
        .collect();
    Ok(json!(results))
}

fn assembled_view(scenario: &ProviderScenario, redactor: &Redactor) -> Value {
    let spec = ResponseSpec {
        model: scenario.model.clone(),
        hidden_params: scenario.hidden_params.clone(),
    };
    let mut response = fixture_response(&spec);
    let selector = FixtureSelector {
        mode: scenario.selector,
    };
    let projection = FixtureProjection {
        mode: scenario.projection,
        calls: std::sync::Mutex::new(Vec::new()),
    };
    let context = ProviderMetadataContext {
        custom_llm_provider: scenario.custom_llm_provider.as_deref(),
        redactor,
        resolver: &PrefixResolver,
        selector: &selector,
        projection: Some(&projection),
    };
    apply_provider_metadata(&mut response, &scenario.chunks, &context);
    let calls = projection.calls.into_inner().unwrap_or_default();
    json!({"response": response_view(&response), "calls": calls})
}

fn failure(message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

struct PrefixResolver;

impl ProviderResolution for PrefixResolver {
    fn provider_for_model(&self, model: &str) -> Result<ProviderId, ResolutionError> {
        let Some((prefix, _rest)) = model.split_once('/') else {
            return Err(ResolutionError::MissingProvider {
                model: model.to_owned(),
            });
        };
        prefix
            .parse::<ProviderId>()
            .map_err(|_error| ResolutionError::MissingProvider {
                model: model.to_owned(),
            })
    }
}

struct FixtureSelector {
    mode: SelectorFixture,
}

impl ChatConfigSelection for FixtureSelector {
    fn select_chat_config(
        &self,
        model: &str,
        provider: ProviderId,
    ) -> Result<Option<ChatConfigTag>, ChatConfigError> {
        match self.mode {
            SelectorFixture::None => Ok(None),
            SelectorFixture::Raise => Err(ChatConfigError::MissingJsonRecord { provider }),
            SelectorFixture::Table => selected_tag(model, provider),
        }
    }
}

fn selected_tag(
    model: &str,
    provider: ProviderId,
) -> Result<Option<ChatConfigTag>, ChatConfigError> {
    let request = ChatConfigRequest {
        model,
        provider,
        base_model: None,
        families: &Probe,
        bedrock: BedrockFacts {
            route: BedrockRoute::ChatCompletions,
            invoke_provider: None,
            base_model: "",
        },
        vertex: VertexModelLists {
            mistral_models: &[],
            ai21_models: &[],
        },
        json_providers: &NoJsonProviders,
    };
    let mut table = ProviderConfigTable { rows: None };
    select_chat_config(&request, &mut table).map(|selection| selection.map(|chosen| chosen.tag))
}

struct Probe;

impl ModelFamilyProbe for Probe {
    fn openai_o_series(&self, _model: &str) -> bool {
        false
    }

    fn openai_gpt_5(&self, _model: &str) -> bool {
        false
    }

    fn openai_catalog_model(&self, _model: &str) -> bool {
        true
    }

    fn azure_gpt_5(&self, _model: &str) -> bool {
        false
    }
}

struct NoJsonProviders;

impl JsonProviderProjection for NoJsonProviders {
    fn lookup(&self, _provider: ProviderId) -> JsonProviderLookup {
        JsonProviderLookup::Absent
    }
}

struct FixtureProjection {
    mode: ProjectionFixture,
    calls: std::sync::Mutex<Vec<Value>>,
}

impl ProviderMetadataProjection for FixtureProjection {
    fn project(
        &self,
        tag: ChatConfigTag,
        response: &mut BaseResponse,
        chunks: &[Chunk],
    ) -> Result<(), ProjectionError> {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(json!({
                "config": tag.class_name(),
                "model": &response.model,
                "chunks": chunks.len(),
            }));
        }
        match self.mode {
            ProjectionFixture::Record => Ok(()),
            ProjectionFixture::Raise => Err(ProjectionError {
                reason: "fixture projection failure".to_owned(),
            }),
        }
    }
}
