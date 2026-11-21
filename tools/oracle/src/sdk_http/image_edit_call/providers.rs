use std::sync::{Mutex, PoisonError};

use betterlite_context::media_config_selection::image_edit::ImageEditConfigTag;
use betterlite_core::api::image_edit_call::call::ImageEditStep;
use betterlite_core::api::image_edit_call::error::ImageEditError;
use betterlite_core::api::image_edit_call::send::{
    CustomEditHandler, CustomEditLookup, CustomEditProviders, CustomEditRequest, ImageEditProvider,
    ImageEditProviderSource, ImageEditSendRequest,
};
use betterlite_core::api::image_input_options::options::ImageEditOptionalRequestParams;
use betterlite_core::api::image_input_options::selection::{
    ImageEditProviderConfig, OptionalParamsPolicy, optional_params_image_edit,
};
use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::api::image_contracts::records::{ImageFile, ImageParams, ImageResponse};
use serde_json::Number;

use super::openai::OpenAiProvider;
use super::{Client, CustomProvider, CustomRecord, Received, Sent};

// B1120's selection over a config that supports these names and maps them as the OpenAI dict copy
pub(super) struct Options<'names> {
    pub(super) supported: &'names [&'names str],
}

impl ImageEditProviderConfig for Options<'_> {
    type Error = ImageEditError;
    type Mapped = ImageParams;

    fn supported_openai_params(&self, _model: &str) -> Vec<String> {
        self.supported
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    }

    fn map_openai_params(
        &self,
        options: &ImageEditOptionalRequestParams,
        _model: &str,
        _drop_params: bool,
    ) -> Result<ImageParams, ImageEditError> {
        Ok(serde_json::from_slice(&serde_json::to_vec(options)?)?)
    }
}

// the real B512 OpenAI wire, or one recorder standing in for every config B157 selects
pub(super) struct Providers {
    recorded: bool,
    openai: OpenAiProvider,
    recorder: Recorder,
}

impl Providers {
    pub(super) fn new(recorded: bool, client: &ProxiedClient) -> Self {
        Self {
            recorded,
            openai: OpenAiProvider {
                client: client.clone(),
            },
            recorder: Recorder {
                sent: Mutex::new(None),
            },
        }
    }

    pub(super) fn take(&self) -> Option<Sent> {
        self.recorder
            .sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

impl ImageEditProviderSource for Providers {
    fn provider(&self, tag: &ImageEditConfigTag) -> Option<&dyn ImageEditProvider> {
        if self.recorded {
            return Some(&self.recorder);
        }
        match tag {
            ImageEditConfigTag::OpenAi(_) => Some(&self.openai),
            ImageEditConfigTag::HostedVllm(_)
            | ImageEditConfigTag::Azure
            | ImageEditConfigTag::Recraft
            | ImageEditConfigTag::BlackForestLabs
            | ImageEditConfigTag::FalAi(_)
            | ImageEditConfigTag::AzureAi(_)
            | ImageEditConfigTag::Gemini(_)
            | ImageEditConfigTag::LiteLlmProxy
            | ImageEditConfigTag::VertexAi(_)
            | ImageEditConfigTag::Stability(_)
            | ImageEditConfigTag::Bedrock(_)
            | ImageEditConfigTag::OpenRouter(_) => None,
        }
    }
}

// the Python side's FixtureConfig(multipart=False): nothing supported, nothing mapped
struct Recorder {
    sent: Mutex<Option<Sent>>,
}

impl ImageEditProvider for Recorder {
    fn supported_openai_params(&self, _model: &str) -> Vec<String> {
        Vec::new()
    }

    fn use_multipart_form_data(&self) -> bool {
        false
    }

    fn map_edit_options(
        &self,
        options: &ImageEditOptionalRequestParams,
        model: &str,
        policy: &OptionalParamsPolicy,
    ) -> Result<ImageParams, ImageEditError> {
        optional_params_image_edit(&Options { supported: &[] }, model, options, policy)
    }

    fn send(&self, request: ImageEditSendRequest<'_>) -> Result<ImageEditStep<'_>, ImageEditError> {
        // bedrock_image_edit and bfl_image_edit take no extra_body; the generic handler does
        let extra_body = match request.custom_llm_provider {
            "bedrock" | "black_forest_labs" => None,
            _ => request.extra_body.cloned(),
        };
        let sent = Sent {
            provider: request.custom_llm_provider.to_owned(),
            received: received(
                request.model,
                request.images,
                request.prompt,
                request.optional_params,
                request.timeout,
            ),
            extra_headers: request.extra_headers.cloned(),
            extra_body,
            asynchronous: request.asynchronous,
        };
        answered(&self.sent, sent, false)
    }
}

fn received(
    model: &str,
    images: &[ImageFile],
    prompt: Option<&str>,
    optional_params: &ImageParams,
    timeout: Option<f64>,
) -> Received {
    Received {
        model: model.to_owned(),
        images: images
            .iter()
            .map(|file| String::from_utf8_lossy(&file.content).into_owned())
            .collect(),
        prompt: prompt.map(str::to_owned),
        optional_params: optional_params.clone(),
        timeout: timeout.and_then(Number::from_f64),
    }
}

// keeps what a recorder received and answers with an empty ImageResponse, as a coroutine when pending
fn answered<T>(
    slot: &Mutex<Option<T>>,
    record: T,
    pending: bool,
) -> Result<ImageEditStep<'static>, ImageEditError> {
    *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(record);
    let answer = ImageResponse {
        created: 1,
        data: Some(Vec::new()),
        usage: None,
    };
    if pending {
        return Ok(ImageEditStep::Pending(Box::pin(async move { Ok(answer) })));
    }
    Ok(ImageEditStep::Answer(answer))
}

// litellm._custom_providers with the handlers each name's custom_provider_map entries carry
pub(super) struct CustomHandlers {
    entries: Vec<(String, Vec<CustomHandler>)>,
}

impl CustomHandlers {
    pub(super) fn new(providers: &[CustomProvider]) -> Self {
        let entries = providers
            .iter()
            .map(|provider| {
                let handlers = provider
                    .handlers
                    .iter()
                    .map(|name| CustomHandler {
                        name: name.clone(),
                        received: Mutex::new(None),
                    })
                    .collect();
                (provider.name.clone(), handlers)
            })
            .collect();
        Self { entries }
    }

    pub(super) fn take(&self) -> Option<CustomRecord> {
        self.entries
            .iter()
            .flat_map(|(_, handlers)| handlers)
            .find_map(|handler| {
                handler
                    .received
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take()
            })
    }
}

impl CustomEditProviders for CustomHandlers {
    fn lookup(&self, provider: &str) -> CustomEditLookup<'_> {
        let Some((_, handlers)) = self.entries.iter().find(|(name, _)| name == provider) else {
            return CustomEditLookup::Missing;
        };
        match handlers.last() {
            Some(handler) => CustomEditLookup::Handler(handler),
            None => CustomEditLookup::Unmatched,
        }
    }
}

struct CustomHandler {
    name: String,
    received: Mutex<Option<CustomRecord>>,
}

impl CustomEditHandler for CustomHandler {
    fn call(
        &self,
        request: CustomEditRequest<'_>,
        asynchronous: bool,
    ) -> Result<ImageEditStep<'_>, ImageEditError> {
        let record = CustomRecord {
            handler: self.name.clone(),
            received: received(
                request.model,
                request.images,
                request.prompt,
                request.optional_params,
                request.timeout,
            ),
            client: request.client.map(Client::from),
            asynchronous,
        };
        answered(&self.received, record, asynchronous)
    }
}
