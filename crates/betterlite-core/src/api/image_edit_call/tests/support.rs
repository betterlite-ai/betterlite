use std::collections::{BTreeMap, BTreeSet};
use std::mem;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::image_edit_call::Settings;
use betterlite_context::media_config_selection::image_edit::ImageEditConfigTag;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::api::image_contracts::records::{
    ImageData, ImageFile, ImageHeaders, ImageParams, ImageResponse,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use serde_json::Number;

use crate::api::image_edit_call::call::{
    ImageEditCall, ImageEditClientKind, ImageEditStep, ImageInput, PendingEdit,
};
use crate::api::image_edit_call::error::ImageEditError;
use crate::api::image_edit_call::logging::{ImageEditLogger, PreCallRecord};
use crate::api::image_edit_call::options::json_from_value;
use crate::api::image_edit_call::send::{
    CustomEditHandler, CustomEditLookup, CustomEditProviders, CustomEditRequest, ImageEditProvider,
    ImageEditProviderSource, ImageEditSendRequest,
};
use crate::api::image_input_options::options::{
    ImageEditOptionalRequestParams, Quality, ResponseFormat,
};
use crate::api::image_input_options::selection::OptionalParamsPolicy;

// what a configured provider's send received, in the shape the parity review reads
#[derive(Debug)]
pub(super) struct Sent {
    pub(super) model: String,
    pub(super) prompt: Option<String>,
    pub(super) images: Vec<ImageFile>,
    pub(super) mask: Option<ImageFile>,
    pub(super) optional_params: ImageParams,
    pub(super) extra_headers: Option<ImageHeaders>,
    pub(super) extra_body: Option<ImageParams>,
    pub(super) timeout: Option<f64>,
    pub(super) api_key: Option<String>,
    pub(super) api_base: Option<String>,
    pub(super) custom_llm_provider: String,
    pub(super) client: Option<ImageEditClientKind>,
    pub(super) asynchronous: bool,
}

pub(super) struct Provider {
    supported: Vec<String>,
    multipart: bool,
    answer: ImageResponse,
    sends: Mutex<Vec<Sent>>,
    pending: bool,
}

impl Provider {
    pub(super) fn new(supported: &[&str], multipart: bool) -> Self {
        Self {
            supported: supported.iter().map(|name| (*name).to_owned()).collect(),
            multipart,
            answer: response(),
            sends: Mutex::new(Vec::new()),
            pending: false,
        }
    }

    #[must_use]
    pub(super) fn pending(mut self) -> Self {
        self.pending = true;
        self
    }

    pub(super) fn take(&self) -> Vec<Sent> {
        let mut sends = self.sends.lock().unwrap_or_else(PoisonError::into_inner);
        mem::take(&mut *sends)
    }
}

impl ImageEditProvider for Provider {
    fn supported_openai_params(&self, _model: &str) -> Vec<String> {
        self.supported.clone()
    }

    fn use_multipart_form_data(&self) -> bool {
        self.multipart
    }

    fn map_edit_options(
        &self,
        options: &ImageEditOptionalRequestParams,
        _model: &str,
        _policy: &OptionalParamsPolicy,
    ) -> Result<ImageParams, ImageEditError> {
        Ok(mapped(options))
    }

    fn send(&self, request: ImageEditSendRequest<'_>) -> Result<ImageEditStep<'_>, ImageEditError> {
        let record = Sent {
            model: request.model.to_owned(),
            prompt: request.prompt.map(str::to_owned),
            images: request.images.to_vec(),
            mask: request.mask.cloned(),
            optional_params: request.optional_params.clone(),
            extra_headers: request.extra_headers.cloned(),
            extra_body: request.extra_body.cloned(),
            timeout: request.timeout,
            api_key: request.api_key.map(str::to_owned),
            api_base: request.api_base.map(str::to_owned),
            custom_llm_provider: request.custom_llm_provider.to_owned(),
            client: request.client,
            asynchronous: request.asynchronous,
        };
        self.sends
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record);
        if self.pending {
            let answer = self.answer.clone();
            let future: PendingEdit<'_> = Box::pin(async move { Ok(answer) });
            return Ok(ImageEditStep::Pending(future));
        }
        Ok(ImageEditStep::Answer(self.answer.clone()))
    }
}

// the four signature options image_edit maps onto optional_params, in the TypedDict's order
pub(super) fn mapped(options: &ImageEditOptionalRequestParams) -> ImageParams {
    let mut params = ImageParams::new();
    if let Some(mask) = &options.mask {
        params.insert(String::from("mask"), Json::Text(mask.clone()));
    }
    if let Some(count) = options.count {
        params.insert(String::from("n"), Json::Number(Number::from(count)));
    }
    if let Some(quality) = options.quality {
        let text = match quality {
            Quality::High => "high",
            Quality::Medium => "medium",
            Quality::Low => "low",
            Quality::Standard => "standard",
            Quality::Auto => "auto",
        };
        params.insert(String::from("quality"), Json::Text(text.to_owned()));
    }
    if let Some(format) = options.response_format {
        let text = match format {
            ResponseFormat::Url => "url",
            ResponseFormat::B64Json => "b64_json",
        };
        params.insert(String::from("response_format"), Json::Text(text.to_owned()));
    }
    if let Some(size) = &options.size {
        params.insert(String::from("size"), Json::Text(size.clone()));
    }
    if let Some(user) = &options.user {
        params.insert(String::from("user"), Json::Text(user.clone()));
    }
    for (name, value) in &options.extensions {
        params.insert(name.clone(), json_from_value(value));
    }
    params
}

pub(super) struct Source {
    tagged: Vec<(ImageEditConfigTag, Arc<Provider>)>,
}

impl Source {
    fn new(tagged: Vec<(ImageEditConfigTag, Arc<Provider>)>) -> Self {
        Self { tagged }
    }
}

impl ImageEditProviderSource for Source {
    fn provider(&self, tag: &ImageEditConfigTag) -> Option<&dyn ImageEditProvider> {
        self.tagged
            .iter()
            .find(|(candidate, _)| candidate == tag)
            .map(|(_, provider)| provider.as_ref() as &dyn ImageEditProvider)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CustomSent {
    pub(super) handler: String,
    pub(super) asynchronous: bool,
    pub(super) model: String,
    pub(super) images: Vec<(Option<String>, Vec<u8>)>,
    pub(super) prompt: Option<String>,
    pub(super) optional_params: ImageParams,
    pub(super) api_key: Option<String>,
    pub(super) api_base: Option<String>,
    pub(super) timeout: Option<f64>,
    pub(super) client: Option<ImageEditClientKind>,
}

pub(super) struct CustomHandler {
    name: &'static str,
    answer: ImageResponse,
    calls: Mutex<Vec<CustomSent>>,
    pending: bool,
}

impl CustomHandler {
    pub(super) fn new(name: &'static str) -> Self {
        Self {
            name,
            answer: response(),
            calls: Mutex::new(Vec::new()),
            pending: false,
        }
    }

    #[must_use]
    pub(super) fn pending(mut self) -> Self {
        self.pending = true;
        self
    }

    pub(super) fn take(&self) -> Vec<CustomSent> {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        mem::take(&mut *calls)
    }
}

impl CustomEditHandler for CustomHandler {
    fn call(
        &self,
        request: CustomEditRequest<'_>,
        asynchronous: bool,
    ) -> Result<ImageEditStep<'_>, ImageEditError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(CustomSent {
                handler: self.name.to_owned(),
                asynchronous,
                model: request.model.to_owned(),
                images: request
                    .images
                    .iter()
                    .map(|image| (image.filename.clone(), image.content.clone()))
                    .collect(),
                prompt: request.prompt.map(str::to_owned),
                optional_params: request.optional_params.clone(),
                api_key: request.api_key.map(str::to_owned),
                api_base: request.api_base.map(str::to_owned),
                timeout: request.timeout,
                client: request.client,
            });
        if self.pending {
            let answer = self.answer.clone();
            let future: PendingEdit<'_> = Box::pin(async move { Ok(answer) });
            return Ok(ImageEditStep::Pending(future));
        }
        Ok(ImageEditStep::Answer(self.answer.clone()))
    }
}

pub(super) struct CustomSource {
    entries: Vec<(String, Vec<Arc<CustomHandler>>)>,
}

impl CustomSource {
    pub(super) fn new(entries: Vec<(&str, Vec<Arc<CustomHandler>>)>) -> Self {
        Self {
            entries: entries
                .into_iter()
                .map(|(name, handlers)| (name.to_owned(), handlers))
                .collect(),
        }
    }
}

impl CustomEditProviders for CustomSource {
    fn lookup(&self, provider: &str) -> CustomEditLookup<'_> {
        let Some((_, handlers)) = self.entries.iter().find(|(name, _)| name == provider) else {
            return CustomEditLookup::Missing;
        };
        match handlers.last() {
            Some(handler) => CustomEditLookup::Handler(handler.as_ref()),
            None => CustomEditLookup::Unmatched,
        }
    }
}

pub(super) struct NoCustom;

impl CustomEditProviders for NoCustom {
    fn lookup(&self, _provider: &str) -> CustomEditLookup<'_> {
        CustomEditLookup::Missing
    }
}

pub(super) struct RecordingLogger {
    pub(super) records: Vec<PreCallRecord>,
}

impl RecordingLogger {
    pub(super) fn new() -> Self {
        Self {
            records: Vec::new(),
        }
    }
}

impl ImageEditLogger for RecordingLogger {
    fn update_from_kwargs(&mut self, record: PreCallRecord) -> Result<(), PrePostError> {
        self.records.push(record);
        Ok(())
    }
}

pub(super) struct FailingLogger;

impl ImageEditLogger for FailingLogger {
    fn update_from_kwargs(&mut self, _record: PreCallRecord) -> Result<(), PrePostError> {
        Err(PrePostError::RawRequestCaptured)
    }
}

pub(super) struct World {
    catalog: Arc<ModelCatalog>,
    names: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl World {
    pub(super) fn new() -> Self {
        Self {
            catalog: BundledCatalog::default().get(),
            names: BTreeSet::new(),
            environment: BTreeMap::new(),
        }
    }

    pub(super) fn context(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.names,
            custom_providers: &self.names,
            environment: &self.environment,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}

pub(super) fn settings(variables: &[(&str, &str)]) -> Settings {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load("model_list: []")
        .expect("the settings load")
        .sections
        .image_edit_call
}

pub(super) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles")
}

pub(super) fn response() -> ImageResponse {
    ImageResponse {
        created: 1,
        data: Some(vec![ImageData {
            b64_json: Some(String::from("YQ==")),
            revised_prompt: None,
            url: None,
        }]),
        usage: None,
    }
}

pub(super) fn image(name: &str, content: &[u8]) -> ImageFile {
    ImageFile {
        filename: Some(name.to_owned()),
        content: content.to_vec(),
        content_type: Some(String::from("image/png")),
    }
}

pub(super) fn upload() -> ImageEditCall {
    ImageEditCall {
        image: ImageInput::Many(Box::new([image("a.png", b"abc"), image("b.png", b"def")])),
        prompt: Some(String::from("change")),
        model: Some(String::from("openai/gpt-image-1")),
        mask: Some(image("m.png", b"ghi")),
        count: Some(1),
        quality: Some(Quality::High),
        response_format: Some(ResponseFormat::B64Json),
        size: Some(String::from("1024x1024")),
        user: Some(String::from("u")),
        extra_headers: None,
        extra_query: None,
        extra_body: None,
        timeout: None,
        custom_llm_provider: None,
        kwargs: ImageParams::new(),
        client: None,
    }
}

pub(super) fn gpt_image_tag() -> ImageEditConfigTag {
    ImageEditConfigTag::OpenAi(String::from("gpt-image-1"))
}
pub(super) fn custom_call() -> ImageEditCall {
    ImageEditCall {
        image: ImageInput::Many(Box::new([image("a.png", b"abc"), image("b.png", b"def")])),
        prompt: Some(String::from("custom")),
        model: Some(String::from("my-model")),
        mask: None,
        count: None,
        quality: None,
        response_format: None,
        size: None,
        user: None,
        extra_headers: None,
        extra_query: None,
        extra_body: None,
        timeout: None,
        custom_llm_provider: Some(String::from("my-provider")),
        kwargs: ImageParams::from([
            (
                String::from("settings"),
                Json::Object(IndexMap::from([(
                    String::from("strength"),
                    Json::Number(Number::from(1)),
                )])),
            ),
            (String::from("api_key"), Json::Text(String::from("sk-test"))),
            (
                String::from("api_base"),
                Json::Text(String::from("https://host/v1")),
            ),
        ]),
        client: Some(ImageEditClientKind::Sync),
    }
}
