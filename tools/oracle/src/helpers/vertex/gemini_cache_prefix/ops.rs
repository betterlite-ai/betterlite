use std::future::{Future, ready};

use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use betterlite_prompts::message_content::Message;
use betterlite_vertex::cache_prefix::selection::{
    ends_on_supported_turn, separate_cached_messages,
};
use betterlite_vertex::cache_prefix::transform::{CachedContentRequest, cached_content_request};
use betterlite_vertex::model_policy::capabilities::{
    CapabilityKind, CapabilityLookupError, ModelCapabilityLookup, VertexCapabilityProvider,
    get_supports_system_message,
};
use jiff::Timestamp;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    model: String,
    provider: ProviderName,
    supports_system_message: bool,
    cache_key: String,
    project: String,
    location: String,
    now: String,
    disable_transform: bool,
    messages: Vec<Message>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProviderName {
    VertexAi,
    VertexAiBeta,
    Gemini,
}

impl ProviderName {
    const fn capability_provider(self) -> VertexCapabilityProvider {
        match self {
            Self::VertexAi => VertexCapabilityProvider::VertexAi,
            Self::VertexAiBeta => VertexCapabilityProvider::VertexAiBeta,
            Self::Gemini => VertexCapabilityProvider::Gemini,
        }
    }
}

struct FixtureLookup {
    supports: bool,
}

impl ModelCapabilityLookup for FixtureLookup {
    fn supports(
        &self,
        kind: CapabilityKind,
        _model: &str,
        _provider: &str,
    ) -> Result<bool, CapabilityLookupError> {
        match kind {
            CapabilityKind::SystemMessages => Ok(self.supports),
            CapabilityKind::ResponseSchema => Err(CapabilityLookupError {
                message: "the cache-prefix helper asks only for system-message support".to_owned(),
            }),
        }
    }
}

struct OfflineMedia;

impl RemoteMediaFetcher for OfflineMedia {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(Err(ImageFetchError::Rejected {
            url: url.to_owned(),
        }))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        ready(Err(ImageFetchError::Rejected {
            url: urls.join(","),
        }))
    }
}

pub(super) fn run(input: &Input) -> HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|error| {
            Box::new(HelperError {
                class: "RuntimeError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
    let mut results = Vec::with_capacity(input.cases.len());
    for case in &input.cases {
        results.push(case_value(case, &runtime)?);
    }
    Ok(json!({ "results": results }))
}

fn case_value(case: &Case, runtime: &Runtime) -> Result<Value, Box<HelperError>> {
    let now: Timestamp = case.now.parse().or_value_error()?;
    let provider = case.provider.capability_provider();
    let lookup = FixtureLookup {
        supports: case.supports_system_message,
    };
    let (cached, non_cached) = separate_cached_messages(&case.messages, case.disable_transform);
    let eligible = !cached.is_empty() && ends_on_supported_turn(&cached);
    let system_supported = get_supports_system_message(&case.model, provider, &lookup);
    let body = if eligible {
        let request = CachedContentRequest {
            model: &case.model,
            messages: &cached,
            provider,
            cache_key: &case.cache_key,
            project: &case.project,
            location: &case.location,
            disable_transform: case.disable_transform,
            now,
        };
        Some(
            runtime
                .block_on(cached_content_request(&request, &lookup, &OfflineMedia))
                .or_value_error()?,
        )
    } else {
        None
    };
    Ok(json!({
        "cached": cached,
        "non_cached": non_cached,
        "eligible": eligible,
        "system_supported": system_supported,
        "body": body,
    }))
}
