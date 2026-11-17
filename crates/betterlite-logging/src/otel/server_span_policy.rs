use betterlite_config::loader::EnvironmentLookup;
use regex::Regex;

// litellm.integrations.otel.mount reads this standard instrumentation variable when it mounts the app
pub const EXCLUDED_URLS_ENV: &str = "OTEL_PYTHON_FASTAPI_EXCLUDED_URLS";

// high-frequency pollers and static assets, none of them LLM traffic; the order is LiteLLM's
pub const DEFAULT_EXCLUDED_ROUTES: [&str; 11] = [
    "/health",
    "/metrics",
    "/litellm-asset-prefix",
    "/_next",
    "/ui",
    "/swagger",
    "/docs",
    "/redoc",
    "/openapi.json",
    "favicon",
    "/.well-known",
];

// catch-all routes whose route template collapses every upstream endpoint into one span name
pub const PASSTHROUGH_PREFIXES: [&str; 16] = [
    "openai",
    "openai_passthrough",
    "anthropic",
    "azure",
    "azure_ai",
    "bedrock",
    "cohere",
    "cursor",
    "gemini",
    "mistral",
    "vllm",
    "vertex_ai",
    "vertex-ai",
    "assemblyai",
    "eu.assemblyai",
    "milvus",
];

#[derive(Debug, thiserror::Error)]
pub enum ServerSpanPolicyError {
    #[error(
        "invalid excluded URL pattern {pattern:?} in OTEL_PYTHON_FASTAPI_EXCLUDED_URLS: {source}"
    )]
    InvalidExcludedUrl {
        pattern: String,
        #[source]
        source: regex::Error,
    },
}

pub struct ServerSpanPolicy {
    excluded: Vec<Regex>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassthroughSpanName {
    pub name: String,
    pub route: String,
}

impl ServerSpanPolicy {
    // litellm.integrations.otel.mount.instrument_fastapi_app: the gate decides whether the policy exists at all
    pub fn from_gate(
        enabled: bool,
        env: &dyn EnvironmentLookup,
    ) -> Result<Option<Self>, ServerSpanPolicyError> {
        if !enabled {
            return Ok(None);
        }
        let urls = env
            .lookup(EXCLUDED_URLS_ENV)
            .unwrap_or_else(|| DEFAULT_EXCLUDED_ROUTES.join(","));
        Ok(Some(Self {
            excluded: excluded_patterns(&urls)?,
        }))
    }

    #[must_use]
    // ast-grep-ignore: no-one-line-functions - the server layer calls this check on every request
    pub fn is_excluded(&self, path: &str) -> bool {
        self.excluded.iter().any(|pattern| pattern.is_match(path))
    }
}

// an override replaces the whole list; an empty value is forwarded as is, which excludes nothing
fn excluded_patterns(urls: &str) -> Result<Vec<Regex>, ServerSpanPolicyError> {
    if urls.is_empty() {
        return Ok(Vec::new());
    }
    urls.split(',')
        .map(|entry| {
            let pattern = entry.trim();
            Regex::new(pattern).map_err(|source| ServerSpanPolicyError::InvalidExcludedUrl {
                pattern: pattern.to_owned(),
                source,
            })
        })
        .collect()
}

// litellm.integrations.otel.mount._passthrough_span_name_hook, minus the span object it renames
#[must_use]
pub fn passthrough_span_name(method: &str, path: &str) -> Option<PassthroughSpanName> {
    let rest = path.trim_start_matches('/');
    let first_segment = rest.split_once('/').map_or(rest, |(first, _)| first);
    if !PASSTHROUGH_PREFIXES.contains(&first_segment) {
        return None;
    }
    Some(PassthroughSpanName {
        name: format!("{method} {path}").trim().to_owned(),
        route: path.to_owned(),
    })
}

mod tests;
