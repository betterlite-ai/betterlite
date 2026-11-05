use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use tokio::runtime::{Handle, Id};
use tokio::sync::Semaphore;

mod body;
mod chunked;
pub mod error;
mod filter;
pub mod merge;
mod response;
pub mod split;
pub mod wire;

#[path = "presidio_analysis/tests/analyze_payload.rs"]
mod analyze_payload;
#[path = "presidio_analysis/tests/analyze_success.rs"]
mod analyze_success;
#[path = "presidio_analysis/tests/analyzer_http_error.rs"]
mod analyzer_http_error;
#[path = "presidio_analysis/tests/analyzer_non_json.rs"]
mod analyzer_non_json;
#[path = "presidio_analysis/tests/analyzer_unparsable.rs"]
mod analyzer_unparsable;
#[path = "presidio_analysis/tests/binary_search.rs"]
mod binary_search;
#[path = "presidio_analysis/tests/chunk_filter.rs"]
mod chunk_filter;
#[path = "presidio_analysis/tests/chunk_semaphore.rs"]
mod chunk_semaphore;
#[path = "presidio_analysis/tests/chunk_size.rs"]
mod chunk_size;
#[path = "presidio_analysis/tests/chunking.rs"]
mod chunking;
#[path = "presidio_analysis/tests/concurrency.rs"]
mod concurrency;
#[path = "presidio_analysis/tests/empty_text.rs"]
mod empty_text;
#[path = "presidio_analysis/tests/fail_closed.rs"]
mod fail_closed;
#[path = "presidio_analysis/tests/fail_open.rs"]
mod fail_open;
#[path = "presidio_analysis/tests/filter_score.rs"]
mod filter_score;
#[path = "presidio_analysis/tests/forward_progress.rs"]
mod forward_progress;
#[path = "presidio_analysis/tests/json_escaped_len.rs"]
mod json_escaped_len_cases;
#[path = "presidio_analysis/tests/merge_chunks.rs"]
mod merge_chunks;
#[path = "presidio_analysis/tests/merge_sort.rs"]
mod merge_sort;
#[path = "presidio_analysis/tests/shared.rs"]
mod shared;
#[path = "presidio_analysis/tests/text_split.rs"]
mod text_split;

use body::read_capped_body;
use chunked::analyze_text_chunked;
use error::PresidioAnalysisError;
use response::post_analyze;
use split::json_escaped_len;
use wire::{AnalyzeBody, AnalyzeOutcome, AnalyzeRequest, AnalyzeResponseItem, MockRedactedText};

pub const DEFAULT_CHUNK_SIZE_BYTES: usize = 500_000;
pub const CHUNK_OVERLAP_CHARS: usize = 4096;
pub const CHUNK_CONCURRENCY: usize = 8;
// aiohttp's default total timeout, used when the guardrail sets none
pub const DEFAULT_ANALYZE_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
const ERROR_SAMPLE_CHARS: usize = 200;

#[derive(Debug, Clone)]
pub struct AnalyzerSettings {
    pub guardrail_name: String,
    // ends with "/"; the lifecycle task normalizes it from PRESIDIO_ANALYZER_API_BASE
    pub analyzer_api_base: String,
    pub language: String,
    pub ad_hoc_recognizers: Option<Vec<GuardrailValue>>,
    // keys of pii_entities_config, in config order
    pub pii_entities: Vec<String>,
    pub output_parse_pii: bool,
    pub apply_to_output: bool,
    pub score_thresholds: BTreeMap<String, f64>,
    pub entities_deny_list: Vec<String>,
    pub chunk_size_bytes: usize,
    pub timeout: Duration,
    pub mock_redacted_text: Option<MockRedactedText>,
}

#[derive(Debug, Clone, Copy)]
pub struct RequestOverrides<'request> {
    pub language: Option<&'request str>,
    // get_guardrail_dynamic_request_body_params is B104's; its result arrives here
    pub dynamic_params: &'request BTreeMap<String, GuardrailValue>,
}

#[derive(Debug)]
pub struct PresidioAnalyzer {
    pub settings: AnalyzerSettings,
    pub client: reqwest::Client,
    pub chunk_semaphores: Mutex<HashMap<Id, Arc<Semaphore>>>,
}

pub async fn json_body(response: reqwest::Response) -> Result<AnalyzeBody, PresidioAnalysisError> {
    let bytes = read_capped_body(response, MAX_RESPONSE_BYTES).await?;
    let raw = serde_json::from_slice(&bytes)
        .map_err(|source| PresidioAnalysisError::ResponseJson { source })?;
    AnalyzeBody::from_raw(raw).map_err(|source| PresidioAnalysisError::ResponseJson { source })
}

impl PresidioAnalyzer {
    // LiteLLM's _get_session_iterator locks and pools aiohttp sessions per thread and loop; the shared reqwest client needs neither.
    #[must_use]
    pub fn analyze_request_payload(
        &self,
        text: &str,
        request: &RequestOverrides<'_>,
    ) -> AnalyzeRequest {
        let settings = &self.settings;
        let dynamic = request.dynamic_params;
        AnalyzeRequest {
            text: (!dynamic.contains_key("text")).then(|| text.to_owned()),
            language: (!dynamic.contains_key("language")).then(|| {
                request
                    .language
                    .map_or_else(|| settings.language.clone(), str::to_owned)
            }),
            ad_hoc_recognizers: settings
                .ad_hoc_recognizers
                .clone()
                .filter(|_| !dynamic.contains_key("ad_hoc_recognizers")),
            entities: (!settings.pii_entities.is_empty() && !dynamic.contains_key("entities"))
                .then(|| settings.pii_entities.clone()),
            dynamic_params: dynamic.clone(),
        }
    }

    pub async fn analyze_text(
        &self,
        text: &str,
        request: &RequestOverrides<'_>,
    ) -> Result<AnalyzeOutcome, PresidioAnalysisError> {
        // A single character cannot be split further, so it always takes the single-call path.
        if self.settings.mock_redacted_text.is_none()
            && text.chars().nth(1).is_some()
            && json_escaped_len(text.chars()) > self.settings.chunk_size_bytes
        {
            return analyze_text_chunked(self, text, request)
                .await
                .map(AnalyzeOutcome::Items);
        }
        if let Some(mock) = &self.settings.mock_redacted_text
            && !text.trim().is_empty()
        {
            return Ok(AnalyzeOutcome::Redacted(mock.clone()));
        }
        self.analyze_single(text, request)
            .await
            .map(AnalyzeOutcome::Items)
    }

    pub(crate) async fn analyze_single(
        &self,
        text: &str,
        request: &RequestOverrides<'_>,
    ) -> Result<Vec<AnalyzeResponseItem>, PresidioAnalysisError> {
        // Blank text is common in tool calls with empty assistant content, and Presidio rejects it.
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }
        let payload = self.analyze_request_payload(text, request);
        post_analyze(self, &payload).await
    }

    // One semaphore per runtime, shared by every chunked call on this analyzer, so concurrent requests together stay under the bound.
    pub fn chunk_semaphore(&self) -> Result<Arc<Semaphore>, PresidioAnalysisError> {
        let runtime = Handle::try_current()
            .map_err(|source| PresidioAnalysisError::NoRuntime { source })?
            .id();
        let mut semaphores = self
            .chunk_semaphores
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let semaphore = Arc::clone(
            semaphores
                .entry(runtime)
                .or_insert_with(|| Arc::new(Semaphore::new(CHUNK_CONCURRENCY))),
        );
        drop(semaphores);
        Ok(semaphore)
    }
}
