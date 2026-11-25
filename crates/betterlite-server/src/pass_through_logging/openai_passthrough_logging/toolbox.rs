use std::str::from_utf8;
use std::sync::Arc;

use betterlite_core::openai_pricing::embedding_cost::embedding_cost as priced_embedding;
use betterlite_core::openai_pricing::image_cost::{
    ImageCostRequest, image_editing_cost, image_generation_cost,
};
use betterlite_core::openai_pricing::responses_api::{
    ResponsesCostRequest, responses_with_cost as priced_responses,
};
use betterlite_core::openai_pricing::{CompletionCost, CompletionCostRequest, CostedResponse};
use betterlite_core::openai_streaming_assembly::{
    RelayAssemblyRequest, RelayTokenCounter, complete_streaming_response,
};
use betterlite_logging::logging_pre_post::env::{DebugSettings, GlobalFlags, LogSink, PrePostEnv};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_success::ports::StandardPayloadBuilder;
use betterlite_logging::passthrough_handler::types::StreamedResponse;
use betterlite_logging::verbose_hooks::VerboseHooks;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::price_lookup::PriceLookup;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::compatible_wire::{ReplyOptions, decode_compatible_chat_reply};
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_openai::response_events::event_decode::parse_terminal_response;
use betterlite_openai::response_events::{RawResponse, ReplyBody, ResponsesReply};
use betterlite_redact::secret::Redactor;
use betterlite_redact::structured::Payload;
use betterlite_streaming::streamed_response_assembly::response::Response;
use futures_util::FutureExt as _;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use uuid::Uuid;

use super::super::gemini_passthrough_logging::assembly::{NoCosts, SilentConsole, SilentLogger};
use super::super::response_costing::{ResponseCostRequest, ResponseCostService};
use super::tools::{
    ChatTransformRequest, ImageEndpoint, OpenAiLoggingTools, OpenAiToolsError, PricedResponse,
    ResponsesRequest, StandardObjectRequest, reencode,
};

// B8931 fills the ports with the proxy's own cost, price, payload and token services.
pub struct OpenAiLoggingToolbox {
    pub chat_cost: Arc<dyn ResponseCostService>,
    pub priced_cost: Arc<dyn CompletionCost + Send + Sync>,
    pub prices: Arc<dyn PriceLookup + Send + Sync>,
    pub builder: Arc<dyn StandardPayloadBuilder>,
    pub counter: Arc<dyn RelayTokenCounter>,
    pub redactor: Arc<Redactor>,
    pub log: Arc<dyn LogSink>,
    pub debug: DebugSettings,
    pub globals: GlobalFlags,
    pub decision: Decision,
    pub identify: fn() -> Uuid,
    pub clock: fn() -> i64,
}

impl OpenAiLoggingTools for OpenAiLoggingToolbox {
    // OpenAIConfig.transform_response reads only the raw reply, so the request's other arguments go unread.
    fn chat_response(&self, request: &ChatTransformRequest<'_>) -> Result<Json, OpenAiToolsError> {
        let options = ReplyOptions {
            json_mode: false,
            provider: None,
            base_model: None,
            identifier: self.identify,
            clock: self.clock,
        };
        let reply = decode_compatible_chat_reply(request.upstream.body, &options)
            .map_err(OpenAiToolsError::Chat)?;
        reencode(&reply.response)
    }

    fn embedding_response(
        &self,
        response_body: &JsonObject,
    ) -> Result<JsonObject, OpenAiToolsError> {
        let embedding: EmbeddingResponse = reencode(response_body)?;
        reencode(&embedding)
    }

    // _calculate_embeddings_cost: any failure logs and bills zero so the spend log is still written.
    fn embedding_cost(
        &self,
        response: &JsonObject,
        model: &str,
        custom_llm_provider: &str,
    ) -> Decimal {
        match reencode::<JsonObject, EmbeddingResponse>(response) {
            Ok(embedding) => priced_embedding(
                &embedding,
                model,
                custom_llm_provider,
                self.priced_cost.as_ref(),
            ),
            Err(error) => {
                tracing::warn!(
                    "Error calculating embeddings cost for model {model}, logging spend with cost 0: {error}"
                );
                Decimal::ZERO
            }
        }
    }

    fn image_cost(
        &self,
        endpoint: ImageEndpoint,
        model: &str,
        request_body: &JsonObject,
    ) -> Decimal {
        let request = ImageCostRequest {
            model,
            request_body,
        };
        match endpoint {
            ImageEndpoint::Generation => image_generation_cost(&request, self.prices.as_ref()),
            ImageEndpoint::Editing => image_editing_cost(&request, self.prices.as_ref()),
        }
    }

    // The relayed reply carries no headers here, so the transform sees an empty header map.
    fn responses_with_cost(
        &self,
        request: &ResponsesRequest<'_>,
        call_log: &mut LoggingState,
    ) -> Result<(Json, Decimal), OpenAiToolsError> {
        let text = from_utf8(request.upstream.body).map_err(OpenAiToolsError::Text)?;
        let headers = IndexMap::new();
        let reply = RawResponse {
            status: request.upstream.status_code,
            headers: &headers,
            text,
        };
        let env = PrePostEnv {
            debug: self.debug,
            globals: self.globals,
            decision: self.decision,
            redactor: self.redactor.as_ref(),
            log: self.log.as_ref(),
            input_callbacks: &[],
            sentry: None,
            logger_fn: None,
            now: request.now,
        };
        let costed = ResponsesCostRequest {
            reply: &reply,
            model: request.model,
            custom_llm_provider: request.custom_llm_provider,
            now: request.now.timestamp(),
            state: call_log,
            env: &env,
        };
        let (built, cost) = priced_responses(costed, self.priced_cost.as_ref())
            .map_err(OpenAiToolsError::Responses)?;
        let response: Json = match &built.response {
            ReplyBody::Typed(typed) => reencode(typed.as_ref())?,
            ReplyBody::Lax(fields) => Json::Object(fields.clone()),
        };
        Ok((response, cost))
    }

    fn terminal_response(&self, all_chunks: &[String]) -> Result<Option<Json>, OpenAiToolsError> {
        parse_terminal_response(all_chunks)
            .map(|terminal| reencode(&terminal))
            .transpose()
    }

    // The assembler is async but its counter and cost ports answer at once, so one poll finishes it.
    fn stream_response(
        &self,
        all_chunks: &[String],
        model: &str,
    ) -> Result<Option<StreamedResponse>, OpenAiToolsError> {
        let request = RelayAssemblyRequest {
            chunks: all_chunks,
            model,
            messages: None,
            counter: self.counter.as_ref(),
            costs: &NoCosts,
            identify: self.identify,
            clock: self.clock,
        };
        let hooks = VerboseHooks {
            logger: SilentLogger,
            console: SilentConsole,
            set_verbose: Payload::Bool(false),
            redactor: Arc::clone(&self.redactor),
        };
        let assembled = complete_streaming_response(&request, &hooks)
            .now_or_never()
            .ok_or(OpenAiToolsError::Pending)?;
        match assembled {
            None => Ok(None),
            Some(Response::Chat(chat)) => {
                Ok(Some(StreamedResponse::Model(reencode(chat.as_ref())?)))
            }
            Some(Response::Text(text)) => {
                Ok(Some(StreamedResponse::TextCompletion(reencode(&text)?)))
            }
        }
    }

    fn completion_cost(&self, request: &PricedResponse<'_>) -> Result<Decimal, OpenAiToolsError> {
        let Some(call_type) = request.call_type else {
            let costed = ResponseCostRequest {
                response: request.response,
                model: request.model,
                custom_llm_provider: Some(request.custom_llm_provider),
                custom_pricing: None,
                router_model_id: None,
            };
            return self
                .chat_cost
                .completion_cost(&costed)
                .map_err(OpenAiToolsError::Cost);
        };
        let terminal: ResponsesApiResponse = reencode(request.response)?;
        let reply = ResponsesReply {
            response: ReplyBody::Typed(Box::new(terminal)),
            headers: IndexMap::new(),
            additional_headers: IndexMap::new(),
        };
        self.priced_cost
            .completion_cost(&CompletionCostRequest {
                response: CostedResponse::Responses(&reply),
                model: request.model,
                custom_llm_provider: request.custom_llm_provider,
                call_type,
            })
            .map_err(OpenAiToolsError::ResponsesCost)
    }

    // get_standard_logging_object_payload answers None on any failure, an encoding one included.
    fn standard_logging_object(
        &self,
        request: &StandardObjectRequest<'_>,
        call_log: &LoggingState,
    ) -> Option<Json> {
        let payload = self.builder.build(
            call_log,
            request.response,
            request.start_time,
            request.end_time,
        )?;
        match reencode(&payload) {
            Ok(object) => Some(object),
            Err(error) => {
                tracing::error!("The standard logging object could not be encoded: {error}");
                None
            }
        }
    }
}
