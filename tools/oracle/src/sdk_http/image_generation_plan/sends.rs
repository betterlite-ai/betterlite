use betterlite_core::api::image_generation_call::error::provider_failure;
use betterlite_core::api::image_generation_call::operations::{BranchAnswer, OpenAiRequest};
use betterlite_error::dispatch::RawException;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::api::image_contracts::records::{
    ImageData, ImageInputTokens, ImageResponse, ImageUsage,
};
use betterlite_openai::image_generation::call::ImageGenerationCall as OpenAiCall;
use betterlite_openai::image_generation::error::ImageGenerationFailure;
use betterlite_openai::image_generation::logging::{
    ImageGenerationLogger, PostCallRecord, PreCallRecord,
};
use betterlite_openai::image_generation::reply::ImageGenerationReply;
use betterlite_openai::image_generation::request::{
    begin_image_generation, finish_image_generation, send_image_generation,
};

// a branch this adapter does not wire fails the case as a ValueError, like the missing handler it stands in for
pub(super) fn unsupported(model: Option<&str>) -> Box<RawException> {
    Box::new(RawException {
        model: model.unwrap_or_default().to_owned(),
        custom_llm_provider: String::new(),
        error_str: String::from(
            "the image_generation_plan oracle adapter wires only the openai branch",
        ),
        status_code: None,
        status_code_is_synthesized: false,
        message: None,
        body: None,
        exception_type: String::from("ValueError"),
    })
}

struct SilentLogger;

impl ImageGenerationLogger for SilentLogger {
    fn pre_call(&mut self, _record: PreCallRecord) -> Result<(), PrePostError> {
        Ok(())
    }

    fn post_call(&mut self, _record: PostCallRecord) {}
}

// the openai image handler B998 sends, over a real connection to the case's mock
pub(super) async fn openai(
    request: OpenAiRequest<'_>,
    client: &ProxiedClient,
) -> Result<BranchAnswer, Box<RawException>> {
    let call = OpenAiCall {
        model: request.model.unwrap_or_default().to_owned(),
        prompt: request.prompt.to_owned(),
        optional_params: request.optional_params.clone(),
        api_key: request.api_key.cloned(),
        api_base: request.api_base.map(str::to_owned),
        organization: request.organization.map(str::to_owned),
        headers: Some(request.headers.clone()),
        timeout: request.timeout,
        delegate_async: request.asynchronous,
        now_seconds: request.now_seconds,
    };
    let mut logger = SilentLogger;
    let model = request.model.unwrap_or_default();
    let prepared =
        begin_image_generation(call, &mut logger).map_err(|failure| failure_of(&failure, model))?;
    let sent = send_image_generation(&prepared, client).await;
    let outcome = finish_image_generation(&prepared, sent, &mut logger)
        .map_err(|failure| failure_of(&failure, model))?;
    Ok(BranchAnswer {
        response: reply_response(&outcome.reply),
        hidden_model: None,
    })
}

fn reply_response(reply: &ImageGenerationReply) -> ImageResponse {
    ImageResponse {
        created: reply.created,
        data: Some(
            reply
                .data
                .iter()
                .map(|datum| ImageData {
                    b64_json: datum.b64_json.clone(),
                    revised_prompt: datum.revised_prompt.clone(),
                    url: datum.url.clone(),
                })
                .collect(),
        ),
        usage: Some(ImageUsage {
            input_tokens: reply.usage.input_tokens,
            input_tokens_details: ImageInputTokens {
                image_tokens: reply.usage.input_tokens_details.image_tokens,
                text_tokens: reply.usage.input_tokens_details.text_tokens,
            },
            output_tokens: reply.usage.output_tokens,
            total_tokens: reply.usage.total_tokens,
        }),
    }
}

fn failure_of(failure: &ImageGenerationFailure, model: &str) -> Box<RawException> {
    Box::new(provider_failure(
        failure.status().unwrap_or(500),
        &failure.to_string(),
        model,
        "openai",
        "OpenAIError",
    ))
}
