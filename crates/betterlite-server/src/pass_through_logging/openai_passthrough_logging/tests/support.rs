use std::sync::{Arc, Mutex, PoisonError};

use betterlite_core::openai_streaming_assembly::RelayTokenCounter;
use betterlite_core::routes::CallType;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::logging_pre_post::env::{DebugSettings, GlobalFlags, LogRecord, LogSink};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_success::ports::StandardPayloadBuilder;
use betterlite_logging::passthrough_handler::types::StreamedResponse;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::tiered_pricing::Scalar;
use betterlite_redact::secret::Redactor;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message;
use chrono::{DateTime, FixedOffset};
use rust_decimal::Decimal;
use serde_json::Number;
use uuid::Uuid;

use super::super::toolbox::OpenAiLoggingToolbox;
use super::super::tools::{
    ChatTransformRequest, ImageEndpoint, OpenAiLoggingTools, OpenAiToolsError, PricedResponse,
    ResponsesRequest, StandardObjectRequest,
};
use super::super::{OpenAiPassthroughError, OpenAiPassthroughLogger};
use crate::pass_through_logging::base_passthrough_logging::tests::support::{
    END, FakePayload, FixedCost, START, logging_state, object, timestamp,
};
use crate::pass_through_logging::base_passthrough_logging::{
    CollectedChunksInput, PassthroughChatInput, PassthroughOutcome, UpstreamChatReply,
};
use crate::pass_through_logging::cohere_passthrough_logging::tests::support::RecordingPricing;
use crate::pass_through_logging::response_payload::ResponseLoggingKwargs;

pub(crate) const CHAT: &str = "https://api.openai.com/v1/chat/completions";
pub(crate) const EMBEDDINGS: &str = "https://api.openai.com/v1/embeddings";
pub(crate) const IMAGES: &str = "https://api.openai.com/v1/images/generations";
pub(crate) const EDITS: &str = "https://api.openai.com/v1/images/edits";
pub(crate) const RESPONSES: &str = "https://api.openai.com/v1/responses";

// One recorded call into the tools, with the arguments the handler chose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Call {
    Chat {
        model: String,
        messages: Json,
        optional_params: Json,
        request_data: JsonObject,
        json_mode: bool,
        litellm_params: JsonObject,
    },
    Embedding,
    EmbeddingCost {
        model: String,
        provider: String,
    },
    ImageCost {
        endpoint: ImageEndpoint,
        model: String,
        request_body: JsonObject,
    },
    Responses {
        model: String,
        provider: String,
    },
    Terminal,
    Stream {
        model: String,
    },
    Cost {
        model: String,
        provider: String,
        call_type: Option<CallType>,
        response: Json,
    },
    // The cost the call log held when the standard object was built.
    Standard {
        response: Json,
        logged_cost: Option<Json>,
    },
}

// The tools as the handlers see them: a None answer is a failure of that call.
pub(crate) struct FakeTools {
    pub(crate) chat: Option<Json>,
    pub(crate) embedding: Option<JsonObject>,
    pub(crate) embedding_price: Decimal,
    pub(crate) image_price: Decimal,
    pub(crate) responses: Option<(Json, Decimal)>,
    pub(crate) terminal: Option<Option<Json>>,
    pub(crate) streamed: Option<Option<StreamedResponse>>,
    pub(crate) price: Option<Decimal>,
    pub(crate) standard: Option<Json>,
    pub(crate) calls: Mutex<Vec<Call>>,
}

impl FakeTools {
    pub(crate) fn working() -> Self {
        Self {
            chat: Some(object_json(r#"{"id": "chatcmpl-1", "choices": []}"#)),
            embedding: Some(object(
                r#"{"object": "list", "data": [], "model": "embed-1"}"#,
            )),
            embedding_price: Decimal::new(2, 3),
            image_price: Decimal::new(4, 2),
            responses: Some((object_json(r#"{"id": "resp-1"}"#), Decimal::new(1, 2))),
            terminal: Some(Some(object_json(r#"{"id": "resp-2"}"#))),
            streamed: Some(Some(StreamedResponse::Model(object_json(
                r#"{"id": "s-1"}"#,
            )))),
            price: Some(Decimal::new(15, 4)),
            standard: Some(object_json(r#"{"status": "success"}"#)),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<Call> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, call: Call) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }
}

fn failed() -> OpenAiToolsError {
    OpenAiToolsError::Pending
}

impl OpenAiLoggingTools for FakeTools {
    fn chat_response(&self, request: &ChatTransformRequest<'_>) -> Result<Json, OpenAiToolsError> {
        self.record(Call::Chat {
            model: request.model.to_owned(),
            messages: request.messages.clone(),
            optional_params: request.optional_params.clone(),
            request_data: request.request_data.clone(),
            json_mode: request.json_mode,
            litellm_params: request.litellm_params.clone(),
        });
        self.chat.clone().ok_or_else(failed)
    }

    fn embedding_response(
        &self,
        _response_body: &JsonObject,
    ) -> Result<JsonObject, OpenAiToolsError> {
        self.record(Call::Embedding);
        self.embedding.clone().ok_or_else(failed)
    }

    fn embedding_cost(&self, _response: &JsonObject, model: &str, provider: &str) -> Decimal {
        self.record(Call::EmbeddingCost {
            model: model.to_owned(),
            provider: provider.to_owned(),
        });
        self.embedding_price
    }

    fn image_cost(
        &self,
        endpoint: ImageEndpoint,
        model: &str,
        request_body: &JsonObject,
    ) -> Decimal {
        self.record(Call::ImageCost {
            endpoint,
            model: model.to_owned(),
            request_body: request_body.clone(),
        });
        self.image_price
    }

    fn responses_with_cost(
        &self,
        request: &ResponsesRequest<'_>,
        _call_log: &mut LoggingState,
    ) -> Result<(Json, Decimal), OpenAiToolsError> {
        self.record(Call::Responses {
            model: request.model.to_owned(),
            provider: request.custom_llm_provider.to_owned(),
        });
        self.responses.clone().ok_or_else(failed)
    }

    fn terminal_response(&self, _all_chunks: &[String]) -> Result<Option<Json>, OpenAiToolsError> {
        self.record(Call::Terminal);
        self.terminal.clone().ok_or_else(failed)
    }

    fn stream_response(
        &self,
        _all_chunks: &[String],
        model: &str,
    ) -> Result<Option<StreamedResponse>, OpenAiToolsError> {
        self.record(Call::Stream {
            model: model.to_owned(),
        });
        self.streamed.clone().ok_or_else(failed)
    }

    fn completion_cost(&self, request: &PricedResponse<'_>) -> Result<Decimal, OpenAiToolsError> {
        self.record(Call::Cost {
            model: request.model.to_owned(),
            provider: request.custom_llm_provider.to_owned(),
            call_type: request.call_type,
            response: request.response.clone(),
        });
        self.price.ok_or_else(failed)
    }

    fn standard_logging_object(
        &self,
        request: &StandardObjectRequest<'_>,
        call_log: &LoggingState,
    ) -> Option<Json> {
        self.record(Call::Standard {
            response: request.response.clone(),
            logged_cost: call_log.model_call_details.get("response_cost").cloned(),
        });
        self.standard.clone()
    }
}

pub(crate) fn object_json(text: &str) -> Json {
    Json::Object(object(text))
}

pub(crate) fn openai_logger(
    tools: &Arc<FakeTools>,
    payload: &Arc<FakePayload>,
) -> OpenAiPassthroughLogger {
    OpenAiPassthroughLogger::new(
        Arc::<FakeTools>::clone(tools),
        Arc::<FakePayload>::clone(payload),
    )
}

pub(crate) fn fallback_payload() -> Arc<FakePayload> {
    Arc::new(FakePayload::new(ResponseLoggingKwargs::default()))
}

// One non-streaming call: the bodies, the call log and the kwargs the adapter would hand over.
pub(crate) struct Relay {
    pub(crate) request: JsonObject,
    pub(crate) response: JsonObject,
    pub(crate) call_log: LoggingState,
    pub(crate) kwargs: ResponseLoggingKwargs,
}

impl Relay {
    pub(crate) fn new(request: &str, response: &str) -> Self {
        Self {
            request: object(request),
            response: object(response),
            call_log: logging_state(),
            kwargs: ResponseLoggingKwargs::default(),
        }
    }

    pub(crate) fn run(
        &mut self,
        logger: &OpenAiPassthroughLogger,
        url_route: &str,
    ) -> Result<PassthroughOutcome, OpenAiPassthroughError> {
        logger.openai_passthrough_handler(PassthroughChatInput {
            upstream: UpstreamChatReply {
                status_code: 200,
                body: b"{}",
            },
            response_body: &self.response,
            call_log: &mut self.call_log,
            url_route,
            start_time: timestamp(START),
            end_time: timestamp(END),
            request_body: &self.request,
            kwargs: self.kwargs.clone(),
        })
    }
}

pub(crate) fn detail(call_log: &LoggingState, key: &str) -> Option<Json> {
    call_log.model_call_details.get(key).cloned()
}

// dall-e-3 at one standard and one hd size, the keys B3635's image pricing reads.
struct ImagePrices;

impl PriceLookup for ImagePrices {
    fn effective_model_info(&self, _model: &str, _provider: Option<&str>) -> Option<PriceRecord> {
        None
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        let price = match key {
            "1024-x-1024/dall-e-3" => 0.04_f64,
            "hd/1024-x-1024/dall-e-3" => 0.08_f64,
            _ => return None,
        };
        let number = Number::from_f64(price).expect("a finite price");
        let mut record = PriceRecord::new();
        record.insert("output_cost_per_image".to_owned(), Scalar::Number(number));
        Some(record)
    }

    fn effective_priced_model(&self, _model: &str, _provider: Option<&str>) -> Option<PricedModel> {
        None
    }
}

struct ZeroCounter;

impl RelayTokenCounter for ZeroCounter {
    fn count_messages(&self, _model: &str, _messages: &[Message]) -> Result<i64, CountError> {
        Ok(0)
    }

    fn count_text(&self, _text: &str) -> Result<i64, CountError> {
        Ok(0)
    }

    fn count_response_text(&self, _model: &str, _text: &str) -> Result<i64, CountError> {
        Ok(0)
    }
}

// The standard payload builder that always builds the stored payload.
pub(crate) struct StoredBuilder;

impl StandardPayloadBuilder for StoredBuilder {
    fn build(
        &self,
        _state: &LoggingState,
        _response: &Json,
        _start_time: DateTime<FixedOffset>,
        _end_time: DateTime<FixedOffset>,
    ) -> Option<StandardLoggingPayload> {
        let stored = include_str!("../fixtures/standard_payload.json");
        Some(serde_json::from_str(stored).expect("the payload fixture parses"))
    }
}

struct QuietSink;

impl LogSink for QuietSink {
    fn emit(&self, _record: LogRecord) {}
}

// The production toolbox over fakes for every port it holds.
pub(crate) fn toolbox(
    pricing: &Arc<RecordingPricing>,
    chat_price: Decimal,
) -> OpenAiLoggingToolbox {
    OpenAiLoggingToolbox {
        chat_cost: Arc::new(FixedCost::new(chat_price)),
        priced_cost: Arc::<RecordingPricing>::clone(pricing),
        prices: Arc::new(ImagePrices),
        builder: Arc::new(StoredBuilder),
        counter: Arc::new(ZeroCounter),
        redactor: Arc::new(Redactor::new(16).expect("a valid redactor")),
        log: Arc::new(QuietSink),
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        identify: Uuid::nil,
        clock: || 1_700_000_000_i64,
    }
}

pub(crate) fn chat_price() -> Decimal {
    Decimal::new(15, 4)
}

pub(crate) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(crate) fn key<'json>(value: &'json Json, name: &str) -> &'json Json {
    let Json::Object(fields) = value else {
        panic!("the value is an object: {value:?}");
    };
    fields.get(name).expect("the object carries the key")
}

// The user written at litellm_params.proxy_server_request.body.user, if any.
pub(crate) fn body_user(litellm_params: &JsonObject) -> Option<Json> {
    let Some(Json::Object(request)) = litellm_params.get("proxy_server_request") else {
        return None;
    };
    let Some(Json::Object(body)) = request.get("body") else {
        return None;
    };
    body.get("user").cloned()
}

pub(crate) fn run(
    fake: FakeTools,
    relay: &mut Relay,
    url_route: &str,
) -> (Arc<FakeTools>, PassthroughOutcome) {
    let tools = Arc::new(fake);
    let logger = openai_logger(&tools, &fallback_payload());
    let outcome = relay.run(&logger, url_route).expect("the handler answers");
    (tools, outcome)
}

pub(crate) fn collect(
    logger: &OpenAiPassthroughLogger,
    call_log: &mut LoggingState,
    request: &str,
    url_route: &str,
) -> PassthroughOutcome {
    let request_body = object(request);
    let lines = vec!["data: chunk".to_owned()];
    logger
        .openai_collected_chunks(
            CollectedChunksInput {
                call_log,
                request_body: &request_body,
                start_time: timestamp(START),
                end_time: timestamp(END),
                all_chunks: &lines,
            },
            url_route,
        )
        .expect("the streaming entry always answers")
}

pub(crate) fn streamed(
    fake: FakeTools,
    request: &str,
    url_route: &str,
) -> (Arc<FakeTools>, LoggingState, PassthroughOutcome) {
    let tools = Arc::new(fake);
    let logger = openai_logger(&tools, &fallback_payload());
    let mut call_log = logging_state();
    let outcome = collect(&logger, &mut call_log, request, url_route);
    (tools, call_log, outcome)
}

pub(crate) const EMBEDDING: &str = r#"{"object": "list", "data": [{"object": "embedding", "embedding": [0.1, 0.2], "index": 0}], "model": "text-embedding-3-small", "usage": {"prompt_tokens": 2, "total_tokens": 2}}"#;

// The production Responses tool over one relayed body.
pub(crate) fn responses_reply(
    body: &[u8],
    pricing: &Arc<RecordingPricing>,
) -> Result<(Json, Decimal), OpenAiToolsError> {
    let tools = toolbox(pricing, chat_price());
    let mut call_log = logging_state();
    let request = ResponsesRequest {
        upstream: &UpstreamChatReply {
            status_code: 200,
            body,
        },
        model: "gpt-4o",
        custom_llm_provider: "openai",
        now: timestamp(END),
    };
    tools.responses_with_cost(&request, &mut call_log)
}
