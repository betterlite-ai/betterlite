#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::image_variations::Settings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::api::image_contracts::records::{
    ImageData, ImageFile, ImageInputTokens, ImageParams, ImageResponse, ImageUsage,
};
use betterlite_openai::image_variations::transport::ImageVariationTransport;

use super::super::call::{
    ImageVariationCall, PendingVariation, VariationAnswer, VariationOptions,
    VariationResponseFormat, VariationStep,
};
use super::super::credentials::VariationPolicies;
use super::super::entry::ImageVariationOperation;
use super::super::error::{ImageVariationError, TopazVariationError};
use super::super::service::ImageVariations;
use super::super::topaz::{TopazVariationExecutor, TopazVariationRequest};

pub(super) const SOURCE: &str = "litellm_settings: {}\n";
pub(super) const SUCCESS_BODY: &[u8] = br#"{"created":1,"data":[{"url":"https://example.test/a.png","b64_json":"YWJj","revised_prompt":"a better bird"}]}"#;
pub(super) const ERROR_BODY: &[u8] =
    br#"{"error":{"message":"nope","type":"invalid_request_error","param":null,"code":null}}"#;
const CATALOG: &str = r#"{
    "dall-e-2": {"litellm_provider": "openai", "mode": "image_generation"},
    "dall-e-3": {"litellm_provider": "openai", "mode": "image_generation"},
    "command-r": {"litellm_provider": "cohere_chat"}
}"#;

pub(super) struct Fixture {
    catalog: ModelCatalog,
    names: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let catalog = ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject)
            .expect("the test catalog parses");
        Self {
            catalog,
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

pub(super) fn settings(source: &str, variables: &[(&str, &str)]) -> Settings {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .expect("the settings load")
        .sections
        .image_variations
}

pub(super) fn service<'context, Transport, Topaz>(
    settings: &'context Settings,
    context: &'context ResolutionContext<'context>,
    transport: &'context Transport,
    topaz: &'context Topaz,
    policies: VariationPolicies,
) -> ImageVariations<'context, Transport, Topaz> {
    ImageVariations {
        settings,
        resolution: context,
        transport,
        topaz,
        policies,
        identify: || uuid::Uuid::from_u128(1),
        clock: || 7,
    }
}

pub(super) fn image() -> ImageFile {
    ImageFile {
        filename: Some("a.png".to_owned()),
        content: b"abc".to_vec(),
        content_type: Some("image/png".to_owned()),
    }
}

pub(super) fn variation_call() -> ImageVariationCall {
    ImageVariationCall {
        image: image(),
        model: None,
        options: VariationOptions::default(),
        api_key: None,
        api_base: None,
        custom_llm_provider: None,
        timeout: None,
        async_call: false,
        metadata: ImageParams::new(),
    }
}

pub(super) fn metadata() -> ImageParams {
    ImageParams::from([("trace".to_owned(), Json::Text("marker".to_owned()))])
}

// the four signature options a variation accepts but never forwards
pub(super) fn ignored_options() -> VariationOptions {
    VariationOptions {
        count: 3,
        response_format: VariationResponseFormat::B64Json,
        size: Some(String::from("1024x1024")),
        user: Some(String::from("u")),
    }
}

pub(super) fn response() -> ImageResponse {
    ImageResponse {
        created: 1,
        data: Some(vec![ImageData {
            b64_json: Some("YWJj".to_owned()),
            revised_prompt: Some("a better bird".to_owned()),
            url: Some("https://example.test/a.png".to_owned()),
        }]),
        usage: Some(ImageUsage {
            input_tokens: 0,
            input_tokens_details: ImageInputTokens {
                image_tokens: 0,
                text_tokens: 0,
            },
            output_tokens: 0,
            total_tokens: 0,
        }),
    }
}

pub(super) fn multipart_parts(request: &OutboundRequest) -> &[MultipartPart] {
    let RequestBody::Multipart { parts, .. } = &request.body else {
        panic!("the variation request is multipart");
    };
    parts
}

pub(super) struct RecordingTransport {
    requests: Mutex<Vec<OutboundRequest>>,
    status: u16,
    body: Vec<u8>,
}

impl RecordingTransport {
    pub(super) fn new(status: u16, body: &[u8]) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            status,
            body: body.to_vec(),
        }
    }

    pub(super) fn take(&self) -> Vec<OutboundRequest> {
        let mut requests = self.requests.lock().unwrap_or_else(PoisonError::into_inner);
        std::mem::take(&mut *requests)
    }
}

impl ImageVariationTransport for RecordingTransport {
    async fn create_variation(
        &self,
        request: OutboundRequest,
    ) -> Result<UpstreamResponse, TransportError> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        Ok(UpstreamResponse {
            status: self.status,
            headers: http::HeaderMap::new(),
            body: self.body.clone(),
        })
    }
}

pub(super) struct RecordingTopaz {
    requests: Mutex<Vec<TopazVariationRequest>>,
    response: Option<ImageResponse>,
}

impl RecordingTopaz {
    pub(super) fn new(response: Option<ImageResponse>) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            response,
        }
    }

    pub(super) fn take(&self) -> Vec<TopazVariationRequest> {
        let mut requests = self.requests.lock().unwrap_or_else(PoisonError::into_inner);
        std::mem::take(&mut *requests)
    }
}

impl TopazVariationExecutor for RecordingTopaz {
    fn execute(
        &self,
        request: TopazVariationRequest,
    ) -> Result<Option<PendingVariation<'_>>, ImageVariationError> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        let Some(response) = self.response.clone() else {
            return Ok(None);
        };
        let pending: PendingVariation<'_> = Box::pin(async move { Ok(response) });
        Ok(Some(pending))
    }
}

pub(super) struct NoTopaz;

impl TopazVariationExecutor for NoTopaz {
    fn execute(
        &self,
        _request: TopazVariationRequest,
    ) -> Result<Option<PendingVariation<'_>>, ImageVariationError> {
        unreachable!("the openai path never reaches topaz")
    }
}

pub(super) struct FailingTopaz;

impl TopazVariationExecutor for FailingTopaz {
    fn execute(
        &self,
        _request: TopazVariationRequest,
    ) -> Result<Option<PendingVariation<'_>>, ImageVariationError> {
        Err(ImageVariationError::Topaz(TopazVariationError {
            status_code: Some(502),
            message: "topaz is down".to_owned(),
        }))
    }
}

pub(super) enum Scripted {
    Response(ImageResponse),
    Mapping(Json),
    Pending(ImageResponse),
    Unexpected(Json),
}

pub(super) struct ScriptedOperation {
    steps: Mutex<VecDeque<Scripted>>,
    calls: Mutex<Vec<ImageVariationCall>>,
    pub(super) awaits: AtomicUsize,
}

impl ScriptedOperation {
    pub(super) fn new(steps: Vec<Scripted>) -> Self {
        Self {
            steps: Mutex::new(steps.into_iter().collect()),
            calls: Mutex::new(Vec::new()),
            awaits: AtomicUsize::new(0),
        }
    }
}

pub(super) fn recorded_calls(operation: &ScriptedOperation) -> Vec<ImageVariationCall> {
    let calls = operation
        .calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    calls.clone()
}

impl ImageVariationOperation for ScriptedOperation {
    fn begin(&self, call: &ImageVariationCall) -> Result<VariationStep<'_>, ImageVariationError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call.clone());
        let next = self
            .steps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        let Some(scripted) = next else {
            return Err(ImageVariationError::InvalidProvider {
                provider: String::from("unscripted"),
            });
        };
        match scripted {
            Scripted::Response(response) => {
                Ok(VariationStep::Answer(VariationAnswer::Response(response)))
            }
            Scripted::Mapping(mapping) => {
                Ok(VariationStep::Answer(VariationAnswer::Mapping(mapping)))
            }
            Scripted::Unexpected(value) => {
                Ok(VariationStep::Answer(VariationAnswer::Unexpected(value)))
            }
            Scripted::Pending(response) => {
                let awaited = &self.awaits;
                Ok(VariationStep::Pending(Box::pin(async move {
                    awaited.fetch_add(1, Ordering::SeqCst);
                    Ok(response)
                })))
            }
        }
    }

    fn inferred_provider(&self, _call: &ImageVariationCall) -> Option<String> {
        None
    }
}

pub(super) struct FailingOperation {
    inferred: Option<String>,
    pub(super) inferences: AtomicUsize,
}

impl FailingOperation {
    pub(super) fn new(inferred: Option<&str>) -> Self {
        Self {
            inferred: inferred.map(str::to_owned),
            inferences: AtomicUsize::new(0),
        }
    }
}

impl ImageVariationOperation for FailingOperation {
    fn begin(&self, _call: &ImageVariationCall) -> Result<VariationStep<'_>, ImageVariationError> {
        Err(ImageVariationError::InvalidProvider {
            provider: String::from("scripted"),
        })
    }

    fn inferred_provider(&self, _call: &ImageVariationCall) -> Option<String> {
        self.inferences.fetch_add(1, Ordering::SeqCst);
        self.inferred.clone()
    }
}
