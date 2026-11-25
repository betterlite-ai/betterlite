use std::mem::take;

use betterlite_core::api::media_relay_logging::spend::decimal_json;
// ast-grep-ignore: no-long-names - B3635's fixed predicate name
use betterlite_core::openai_pricing::route_predicates::is_openai_chat_completions_route;
// ast-grep-ignore: no-long-names - B3635's fixed predicate name
use betterlite_core::openai_pricing::route_predicates::is_openai_image_generation_route;
use betterlite_core::openai_pricing::route_predicates::{
    is_openai_embeddings_route, is_openai_image_editing_route, is_openai_responses_route,
};
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::api::image_contracts::records::{ImageData, ImageResponse, ImageUsage};
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::super::base_passthrough_logging::{PassthroughChatInput, PassthroughOutcome};
use super::tools::{
    ChatTransformRequest, ImageEndpoint, PricedResponse, ResponsesRequest, StandardObjectRequest,
    reencode,
};
use super::user::{openai_end_user, write_end_user};
use super::{OPENAI, OpenAiPassthroughError, OpenAiPassthroughLogger, text_or, write_call_log};

// LiteLLM marks both image endpoints with the generation value; there is no editing one.
const IMAGE_CALL_TYPE: &str = "passthrough-image-generation";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Endpoint {
    ChatCompletions,
    Embeddings,
    ImageGeneration,
    ImageEditing,
    Responses,
}

impl Endpoint {
    // The branches are exclusive and tried in LiteLLM's order.
    fn classify(url_route: &str) -> Option<Self> {
        // ast-grep-ignore: no-long-names - B3635's fixed predicate name
        if is_openai_chat_completions_route(url_route) {
            return Some(Self::ChatCompletions);
        }
        if is_openai_embeddings_route(url_route) {
            return Some(Self::Embeddings);
        }
        // ast-grep-ignore: no-long-names - B3635's fixed predicate name
        if is_openai_image_generation_route(url_route) {
            return Some(Self::ImageGeneration);
        }
        if is_openai_image_editing_route(url_route) {
            return Some(Self::ImageEditing);
        }
        is_openai_responses_route(url_route).then_some(Self::Responses)
    }

    const fn name(self) -> &'static str {
        match self {
            Self::ChatCompletions => "chat_completions",
            Self::Embeddings => "embeddings",
            Self::ImageGeneration => "image_generation",
            Self::ImageEditing => "image_editing",
            Self::Responses => "responses",
        }
    }

    fn logged(self, response: Json) -> PassThroughResponse {
        match self {
            Self::ChatCompletions => PassThroughResponse::ModelResponse(response),
            Self::Embeddings => PassThroughResponse::Embedding(response),
            Self::ImageGeneration | Self::ImageEditing => PassThroughResponse::Image(response),
            Self::Responses => PassThroughResponse::ResponsesApi(response),
        }
    }
}

// The key alone decides, so a request model that is present but null or empty is the empty model.
fn handler_model(
    request_body: &JsonObject,
    response_body: &JsonObject,
) -> Result<String, OpenAiPassthroughError> {
    match request_body.get("model") {
        Some(requested) => text_or(Some(requested), "", "request_body.model"),
        None => text_or(response_body.get("model"), "", "response_body.model"),
    }
}

impl OpenAiPassthroughLogger {
    pub fn openai_passthrough_handler(
        &self,
        mut input: PassthroughChatInput<'_>,
    ) -> Result<PassthroughOutcome, OpenAiPassthroughError> {
        let Some(endpoint) = Endpoint::classify(input.url_route) else {
            return Ok(PassthroughOutcome {
                result: None,
                kwargs: input.kwargs,
            });
        };
        let model = handler_model(input.request_body, input.response_body)?;
        if model.is_empty() {
            tracing::warn!(
                "No model found in request or response for OpenAI passthrough cost tracking"
            );
            return self
                .chat
                .passthrough_chat_handler(input)
                .map_err(OpenAiPassthroughError::from);
        }
        let error = match self.priced(endpoint, &model, &mut input) {
            Ok(outcome) => return Ok(outcome),
            Err(error) => error,
        };
        tracing::error!("Error in OpenAI passthrough cost tracking: {error}");
        match endpoint {
            Endpoint::ChatCompletions => self
                .chat
                .passthrough_chat_handler(input)
                .map_err(OpenAiPassthroughError::from),
            Endpoint::Embeddings
            | Endpoint::ImageGeneration
            | Endpoint::ImageEditing
            | Endpoint::Responses => Ok(PassthroughOutcome {
                result: None,
                kwargs: input.kwargs,
            }),
        }
    }

    // LiteLLM's guarded block: whatever it wrote to the kwargs and the call log stays when a later step fails.
    fn priced(
        &self,
        endpoint: Endpoint,
        model: &str,
        input: &mut PassthroughChatInput<'_>,
    ) -> Result<PassthroughOutcome, OpenAiPassthroughError> {
        let (response, cost) = self.branch(endpoint, model, input)?;
        input.kwargs.response_cost = Some(cost);
        input.kwargs.model = Some(model.to_owned());
        if let Some(user) = openai_end_user(input.kwargs.passthrough_request_body.as_ref()) {
            write_end_user(&mut input.kwargs.litellm_params, &user)?;
            write_end_user(&mut input.call_log.litellm_params, &user)?;
        }
        // The pass-through kwargs never carry a provider, so LiteLLM's kwargs lookup always gives openai.
        write_call_log(&mut *input.call_log, model, OPENAI, cost);
        let standard = StandardObjectRequest {
            response: &response,
            start_time: input.start_time,
            end_time: input.end_time,
        };
        // The non-streaming path builds the object and drops it, as LiteLLM does.
        let _dropped = self
            .tools
            .standard_logging_object(&standard, &*input.call_log);
        tracing::debug!(
            "OpenAI passthrough cost tracking - Endpoint: {}, Model: {model}, Cost: ${cost:.6}",
            endpoint.name()
        );
        Ok(PassthroughOutcome {
            result: Some(endpoint.logged(response)),
            kwargs: take(&mut input.kwargs),
        })
    }

    fn branch(
        &self,
        endpoint: Endpoint,
        model: &str,
        input: &mut PassthroughChatInput<'_>,
    ) -> Result<(Json, Decimal), OpenAiPassthroughError> {
        match endpoint {
            Endpoint::ChatCompletions => self.chat_branch(model, input),
            Endpoint::Embeddings => self.embedding_branch(model, input.response_body),
            Endpoint::ImageGeneration => self.image_branch(ImageEndpoint::Generation, model, input),
            Endpoint::ImageEditing => self.image_branch(ImageEndpoint::Editing, model, input),
            Endpoint::Responses => {
                let request = ResponsesRequest {
                    upstream: &input.upstream,
                    model,
                    custom_llm_provider: OPENAI,
                    now: input.end_time,
                };
                self.tools
                    .responses_with_cost(&request, &mut *input.call_log)
                    .map_err(OpenAiPassthroughError::from)
            }
        }
    }

    fn chat_branch(
        &self,
        model: &str,
        input: &PassthroughChatInput<'_>,
    ) -> Result<(Json, Decimal), OpenAiPassthroughError> {
        let no_messages = Json::List(Vec::new());
        let no_params = Json::Object(JsonObject::new());
        let messages = value_or(input.request_body, "messages", &no_messages);
        let optional_params = value_or(input.request_body, "optional_params", &no_params);
        let request = ChatTransformRequest {
            upstream: &input.upstream,
            model,
            messages,
            optional_params,
            request_data: input.request_body,
            json_mode: json_mode(input.request_body)?,
            litellm_params: &input.kwargs.litellm_params,
        };
        let response = self.tools.chat_response(&request)?;
        let cost = self.tools.completion_cost(&PricedResponse {
            response: &response,
            model,
            custom_llm_provider: OPENAI,
            call_type: None,
        })?;
        Ok((response, cost))
    }

    fn embedding_branch(
        &self,
        model: &str,
        response_body: &JsonObject,
    ) -> Result<(Json, Decimal), OpenAiPassthroughError> {
        let mut response = self.tools.embedding_response(response_body)?;
        let cost = self.tools.embedding_cost(&response, model, OPENAI);
        insert_hidden_cost(&mut response, cost);
        Ok((Json::Object(response), cost))
    }

    // The cost and the call type are written before the response is built, so both stay when the data is malformed.
    fn image_branch(
        &self,
        endpoint: ImageEndpoint,
        model: &str,
        input: &mut PassthroughChatInput<'_>,
    ) -> Result<(Json, Decimal), OpenAiPassthroughError> {
        let cost = self.tools.image_cost(endpoint, model, input.request_body);
        IMAGE_CALL_TYPE.clone_into(&mut input.call_log.call_type);
        let no_images = Json::List(Vec::new());
        let data: Vec<ImageData> = reencode(value_or(input.response_body, "data", &no_images))?;
        // ImageResponse stamps created with the logging time and fills a zero usage; it keeps no model.
        let image = ImageResponse {
            created: input.end_time.timestamp(),
            data: Some(data),
            usage: Some(ImageUsage::default()),
        };
        let mut response: JsonObject = reencode(&image)?;
        insert_hidden_cost(&mut response, cost);
        Ok((Json::Object(response), cost))
    }
}

// Python's `body.get(key) or default`: a missing or null key gives LiteLLM's empty default.
fn value_or<'body>(body: &'body JsonObject, key: &str, fallback: &'body Json) -> &'body Json {
    match body.get(key) {
        None | Some(Json::Null) => fallback,
        Some(given) => given,
    }
}

// request_body["response_format"]["type"] == "json_object"; a missing or null format is no json mode.
fn json_mode(request_body: &JsonObject) -> Result<bool, OpenAiPassthroughError> {
    match request_body.get("response_format") {
        None | Some(Json::Null) => Ok(false),
        Some(Json::Object(format)) => Ok(matches!(
            format.get("type"),
            Some(Json::Text(kind)) if kind == "json_object"
        )),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(OpenAiPassthroughError::FieldType {
                field: "request_body.response_format",
            })
        }
    }
}

// _hidden_params["response_cost"], which keeps the success handler from pricing the response again.
fn insert_hidden_cost(response: &mut JsonObject, cost: Decimal) {
    response.insert(
        "_hidden_params".to_owned(),
        Json::Object(IndexMap::from([(
            "response_cost".to_owned(),
            decimal_json(cost),
        )])),
    );
}
