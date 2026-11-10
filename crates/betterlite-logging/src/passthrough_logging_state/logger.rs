use std::sync::Arc;

use async_trait::async_trait;
use url::Url;

use super::dispatch::{PassThroughLogCall, PassThroughLogDispatch, handle_logging};

pub struct PassThroughEndpointLogging {
    pub injected_log_dispatch: Option<Arc<dyn PassThroughLogDispatch>>,
}

struct ParsedRoute {
    hostname: Option<String>,
    path: String,
}

// urlparse reads an absolute URL; without a scheme the text before ? or # is the path
fn parse_route(url_route: &str) -> ParsedRoute {
    if let Ok(url) = Url::parse(url_route) {
        return ParsedRoute {
            hostname: url.host_str().map(str::to_owned),
            path: url.path().to_owned(),
        };
    }
    let end = url_route.find(['?', '#']).unwrap_or(url_route.len());
    ParsedRoute {
        hostname: None,
        path: url_route.get(..end).unwrap_or_default().to_owned(),
    }
}

impl PassThroughEndpointLogging {
    pub const TRACKED_VERTEX_METHOD_ROUTES: [&str; 9] = [
        "generateContent",
        "streamGenerateContent",
        "predict",
        "rawPredict",
        "streamRawPredict",
        "search",
        "predictLongRunning",
        "embedContent",
        "batchEmbedContents",
    ];
    pub const TRACKED_VERTEX_RESOURCE_ROUTES: [&str; 1] = ["batchPredictionJobs"];
    pub const TRACKED_ANTHROPIC_ROUTES: [&str; 2] = ["/messages", "/v1/messages/batches"];
    pub const TRACKED_COHERE_ROUTES: [&str; 2] = ["/v2/chat", "/v1/embed"];
    pub const TRACKED_LANGFUSE_ROUTES: [&str; 1] = ["/langfuse/"];
    pub const TRACKED_GEMINI_ROUTES: [&str; 3] = [
        "generateContent",
        "streamGenerateContent",
        "predictLongRunning",
    ];
    pub const TRACKED_CURSOR_ROUTES: [&str; 4] =
        ["/v0/agents", "/v0/me", "/v0/models", "/v0/repositories"];
    pub const TRACKED_VERTEX_AI_LIVE_ROUTES: [&str; 1] = ["/vertex_ai/live"];

    #[must_use]
    // ast-grep-ignore: no-one-line-functions
    pub fn log_dispatch(&self) -> &dyn PassThroughLogDispatch {
        self.injected_log_dispatch.as_deref().unwrap_or(self)
    }

    #[must_use]
    pub fn is_anthropic_route(url_route: &str) -> bool {
        Self::TRACKED_ANTHROPIC_ROUTES
            .iter()
            .any(|route| url_route.contains(route))
    }

    #[must_use]
    pub fn is_cohere_route(url_route: &str) -> bool {
        // "/v1/embeddings" also contains "/v1/embed", so only the embeddings route is skipped
        let [chat, embed] = Self::TRACKED_COHERE_ROUTES;
        url_route.contains(chat)
            || (url_route.contains(embed) && !url_route.contains("/v1/embeddings"))
    }

    #[must_use]
    pub fn is_assemblyai_route(url_route: &str) -> bool {
        let parsed = parse_route(url_route);
        if parsed.hostname.as_deref() == Some("api.assemblyai.com") {
            return true;
        }
        parsed.path.contains("/transcript")
    }

    #[must_use]
    // ast-grep-ignore: no-one-line-functions
    pub fn is_comprehend_medical_route(custom_llm_provider: Option<&str>) -> bool {
        custom_llm_provider == Some("comprehendmedical")
    }

    #[must_use]
    // ast-grep-ignore: no-one-line-functions
    pub fn is_azure_speech_route(custom_llm_provider: Option<&str>) -> bool {
        custom_llm_provider == Some("azure_speech")
    }

    #[must_use]
    // ast-grep-ignore: no-one-line-functions
    pub fn is_transcribe_route(custom_llm_provider: Option<&str>) -> bool {
        custom_llm_provider == Some("transcribe")
    }

    #[must_use]
    // ast-grep-ignore: no-one-line-functions
    pub fn is_typesafe_route(custom_llm_provider: Option<&str>) -> bool {
        custom_llm_provider == Some("typesafe")
    }

    #[must_use]
    pub fn is_openrouter_decisions_route(
        url_route: &str,
        custom_llm_provider: Option<&str>,
    ) -> bool {
        custom_llm_provider == Some("openrouter")
            && parse_route(url_route).path.ends_with("/alpha/decisions")
    }

    #[must_use]
    pub fn is_langfuse_route(url_route: &str) -> bool {
        let parsed = parse_route(url_route);
        Self::TRACKED_LANGFUSE_ROUTES
            .iter()
            .any(|route| parsed.path.contains(route))
    }

    #[must_use]
    pub fn is_vertex_ai_live_route(url_route: &str) -> bool {
        if url_route.is_empty() {
            return false;
        }
        Self::TRACKED_VERTEX_AI_LIVE_ROUTES
            .iter()
            .any(|route| url_route.contains(route))
    }

    #[must_use]
    pub fn is_cursor_route(url_route: &str, custom_llm_provider: Option<&str>) -> bool {
        if custom_llm_provider == Some("cursor") {
            return true;
        }
        let parsed = parse_route(url_route);
        if parsed
            .hostname
            .as_deref()
            .is_some_and(|hostname| hostname.contains("api.cursor.com"))
        {
            return true;
        }
        for route in Self::TRACKED_CURSOR_ROUTES {
            if url_route.contains(route) && parsed.path.starts_with("/v0/") {
                return custom_llm_provider == Some("cursor");
            }
        }
        false
    }
}

#[async_trait]
impl PassThroughLogDispatch for PassThroughEndpointLogging {
    async fn dispatch(&self, call: PassThroughLogCall<'_>) {
        handle_logging(call).await;
    }
}
