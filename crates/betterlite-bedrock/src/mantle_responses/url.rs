use crate::mantle_inputs::records::MantleGlobals;
use crate::mantle_inputs::region::{is_mantle_host, resolve_region};
use crate::mantle_responses::config::MantleResponsesConfig;
use crate::mantle_responses::error::MantleResponsesError;

// Longest and most specific first, so a full endpoint URL collapses to its host in one pass.
pub const BASE_SUFFIXES_TO_STRIP: [&str; 5] = [
    "/openai/v1/responses",
    "/v1/responses",
    "/responses",
    "/openai/v1",
    "/v1",
];

const OPENAI_RESPONSES_PATH: &str = "/openai/v1/responses";
const RESPONSES_PATH: &str = "/v1/responses";

#[derive(Debug, Clone, Copy, Default)]
pub struct CompleteUrlParams<'text> {
    pub api_base: Option<&'text str>,
    pub aws_region_name: Option<&'text str>,
}

pub fn complete_url(
    config: &MantleResponsesConfig,
    params: CompleteUrlParams<'_>,
    globals: &MantleGlobals,
) -> Result<String, MantleResponsesError> {
    let region = resolve_region(params.aws_region_name, params.api_base, globals)?;
    let configured = [params.api_base, globals.mantle_api_base.as_deref()]
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty());
    let base = configured.map_or_else(
        || format!("https://bedrock-mantle.{region}.api.aws"),
        str::to_owned,
    );
    let stripped = strip_base_suffix(base.trim_end_matches('/'));
    let host = if is_mantle_host(stripped) {
        format!("https://bedrock-mantle.{region}.api.aws")
    } else {
        stripped.to_owned()
    };
    let path = if config.use_openai_path {
        OPENAI_RESPONSES_PATH
    } else {
        RESPONSES_PATH
    };
    Ok(format!("{host}{path}"))
}

fn strip_base_suffix(base: &str) -> &str {
    for suffix in BASE_SUFFIXES_TO_STRIP {
        if let Some(stripped) = base.strip_suffix(suffix) {
            return stripped;
        }
    }
    base
}
