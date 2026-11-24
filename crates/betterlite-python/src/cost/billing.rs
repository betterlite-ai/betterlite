use betterlite_core::cost::call_price::{CallPriceInput, CustomCostPerToken};
use betterlite_core::cost::completion::args::{ChargeInputs, ResponseParts, ToolInputs};
use betterlite_core::cost::image_dispatch::policy::ImageResponseView;
use betterlite_core::cost::prepare::{CompletionBillingInput, ResponseView};
use betterlite_core::cost::special::replicate::ReplicateStamps;
use betterlite_core::cost::special::request::{ImageParts, LoggingView, TranscriptionDuration};
use betterlite_core::cost::usage::{BillingUsage, ResponseKind};
use betterlite_core::routes::CallType;
use betterlite_models::decisions::Json;
use betterlite_models::tool_fees::billing::BuiltInToolParams;
use betterlite_models::tool_signals::response::{OtherView, ToolBillingResponse};
use betterlite_models::usage_sum::Usage;
use betterlite_openai::api::rerank_contracts::records::RerankBilledUnits;
use indexmap::IndexMap;
use serde::Deserialize;

use crate::cost::convert::{Amount, optional};

// custom_cost_per_token as LiteLLM's CostPerToken declares it: two required rates and two cache rates.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CustomRate {
    pub(crate) input_cost_per_token: Amount,
    pub(crate) output_cost_per_token: Amount,
    #[serde(default)]
    pub(crate) cache_read_input_token_cost: Option<Amount>,
    #[serde(default)]
    // ast-grep-ignore: no-long-names
    pub(crate) cache_creation_input_token_cost: Option<Amount>,
}

impl CustomRate {
    #[must_use]
    pub(crate) const fn cost(&self) -> CustomCostPerToken {
        CustomCostPerToken {
            input_cost_per_token: self.input_cost_per_token.0,
            output_cost_per_token: self.output_cost_per_token.0,
            cache_read_input_token_cost: optional(self.cache_read_input_token_cost),
            // ast-grep-ignore: no-long-names
            cache_creation_cost: optional(self.cache_creation_input_token_cost),
        }
    }
}

// The billed units a rerank response reports, as LiteLLM's RerankBilledUnits holds them.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RerankUnits {
    pub(crate) search_units: Option<i64>,
    pub(crate) total_tokens: Option<i64>,
}

impl RerankUnits {
    #[must_use]
    pub(crate) fn billed(&self) -> RerankBilledUnits {
        RerankBilledUnits {
            search_units: self.search_units.map(Some),
            total_tokens: self.total_tokens.map(Some),
        }
    }
}

// cost_per_token's arguments, each named as LiteLLM's signature names it.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct CallRequest {
    pub(crate) model: Option<String>,
    pub(crate) prompt_tokens: i64,
    pub(crate) completion_tokens: i64,
    pub(crate) response_time_ms: Option<Amount>,
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) region_name: Option<String>,
    pub(crate) prompt_characters: Option<i64>,
    pub(crate) completion_characters: Option<i64>,
    pub(crate) cache_creation_input_tokens: Option<i64>,
    pub(crate) cache_read_input_tokens: Option<i64>,
    pub(crate) custom_cost_per_token: Option<CustomRate>,
    pub(crate) custom_cost_per_second: Option<Amount>,
    pub(crate) number_of_queries: Option<i64>,
    pub(crate) usage: Option<IndexMap<String, Json>>,
    pub(crate) rerank_billed_units: Option<RerankUnits>,
    pub(crate) call_type: Option<CallType>,
    pub(crate) audio_seconds: Option<Amount>,
    pub(crate) service_tier: Option<String>,
    pub(crate) data_residency: Option<String>,
    pub(crate) vertex_location: Option<String>,
    pub(crate) request_model: Option<String>,
}

impl CallRequest {
    // The price input as B068 reads it; the usage object and the billed units are borrowed from the caller.
    #[must_use]
    pub(crate) fn input<'request>(
        &'request self,
        usage: Option<&'request Usage>,
        rerank: Option<&'request RerankBilledUnits>,
    ) -> CallPriceInput<'request> {
        CallPriceInput {
            model: self.model.as_deref(),
            prompt_tokens: self.prompt_tokens,
            completion_tokens: self.completion_tokens,
            response_time_ms: optional(self.response_time_ms),
            custom_llm_provider: self.custom_llm_provider.as_deref(),
            region_name: self.region_name.as_deref(),
            prompt_characters: self.prompt_characters,
            completion_characters: self.completion_characters,
            cache_creation_input_tokens: self.cache_creation_input_tokens,
            cache_read_input_tokens: self.cache_read_input_tokens,
            custom_cost_per_token: self.custom_cost_per_token.as_ref().map(CustomRate::cost),
            custom_cost_per_second: optional(self.custom_cost_per_second),
            number_of_queries: self.number_of_queries,
            usage,
            rerank_billed_units: rerank,
            call_type: self.call_type.unwrap_or(CallType::Completion),
            audio_seconds: self.audio_seconds.unwrap_or_default().0,
            service_tier: self.service_tier.as_deref(),
            data_residency: self.data_residency.as_deref(),
            vertex_location: self.vertex_location.as_deref(),
            vector_response: None,
            ocr_billing: None,
            search_hidden_params: None,
            request_model: self.request_model.as_deref(),
            custom_model_info: None,
            server_tool_projection: None,
        }
    }
}

// The response object as Python dumped it, with _hidden_params carried beside the fields.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ResponseWire {
    pub(crate) kind: Option<ResponseKind>,
    pub(crate) model: Option<String>,
    pub(crate) usage: Option<IndexMap<String, Json>>,
    pub(crate) service_tier: Option<String>,
    pub(crate) hidden_params: Option<IndexMap<String, Json>>,
    pub(crate) created: Option<Amount>,
    pub(crate) ended: Option<Amount>,
}

impl ResponseWire {
    pub(crate) const fn kind(&self) -> ResponseKind {
        match self.kind {
            Some(found) => found,
            None => ResponseKind::Other,
        }
    }

    pub(crate) const fn billing_usage(&self) -> BillingUsage<'_> {
        match self.usage.as_ref() {
            Some(found) => BillingUsage::Mapping(found),
            None => BillingUsage::Absent,
        }
    }

    pub(crate) const fn stamps(&self) -> ReplicateStamps {
        ReplicateStamps {
            created: optional(self.created),
            ended: optional(self.ended),
        }
    }
}

// completion_cost's arguments, each named as LiteLLM's signature names it.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct CompletionRequest {
    pub(crate) model: Option<String>,
    pub(crate) prompt: String,
    pub(crate) messages: Vec<Json>,
    pub(crate) completion: String,
    pub(crate) total_time: Option<Amount>,
    pub(crate) call_type: Option<CallType>,
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) region_name: Option<String>,
    pub(crate) custom_cost_per_token: Option<CustomRate>,
    pub(crate) custom_cost_per_second: Option<Amount>,
    pub(crate) optional_params: Option<IndexMap<String, Json>>,
    pub(crate) custom_pricing: Option<bool>,
    pub(crate) base_model: Option<String>,
    pub(crate) router_model_id: Option<String>,
    pub(crate) service_tier: Option<String>,
    pub(crate) data_residency: Option<String>,
    pub(crate) vertex_location: Option<String>,
    pub(crate) request_model: Option<String>,
    pub(crate) response: Option<ResponseWire>,
}

impl CompletionRequest {
    #[must_use]
    pub(crate) fn billing<'request>(
        &'request self,
        response: Option<&'request ResponseView<'request>>,
    ) -> CompletionBillingInput<'request> {
        let model = self.model.as_deref();
        let prompt = &self.prompt;
        let messages = &self.messages;
        let completion = &self.completion;
        let call_type = self.call_type;
        let custom_llm_provider = self.custom_llm_provider.as_deref();
        let region_name = self.region_name.as_deref();
        let router_model_id = self.router_model_id.as_deref();
        let custom_pricing = self.custom_pricing;
        let base_model = self.base_model.as_deref();
        let optional_params = self.optional_params.as_ref();
        let service_tier = self.service_tier.as_deref();
        CompletionBillingInput {
            model,
            prompt,
            messages,
            completion,
            call_type,
            custom_llm_provider,
            region_name,
            router_model_id,
            custom_pricing,
            base_model,
            optional_params,
            service_tier,
            response,
        }
    }

    // Every response part that belongs to another family stays empty; only the Replicate stamps cross over.
    #[must_use]
    pub(crate) fn charge(&self) -> ChargeInputs<'_> {
        let response = self.response.as_ref();
        let stamps = response.map_or_else(ReplicateStamps::default, ResponseWire::stamps);
        ChargeInputs {
            passthrough_image: false,
            total_time: optional(self.total_time),
            data_residency: self.data_residency.as_deref(),
            vertex_location: self.vertex_location.as_deref(),
            custom_cost_per_token: self.custom_cost_per_token.as_ref().map(CustomRate::cost),
            custom_cost_per_second: optional(self.custom_cost_per_second),
            logging: LoggingView {
                model: self.request_model.as_deref(),
                params: None,
                timing: None,
            },
            response: ResponseParts {
                cost_response: None,
                image: ImageParts {
                    response: ImageResponseView::Other,
                    quality: None,
                    image_count: None,
                    size: None,
                },
                video_usage: None,
                transcription: TranscriptionDuration::default(),
                rerank: None,
                protocol: None,
                replicate: stamps,
                model_response: None,
            },
            ocr_billing: None,
            vector_response: None,
            search_hidden_params: None,
            server_tool_projection: None,
            tools: ToolInputs {
                response: ToolBillingResponse::Other(OtherView { usage: None }),
                params: BuiltInToolParams::default(),
                google_maps_requests: None,
            },
        }
    }
}
