pub mod catalog;
pub mod model_info;
pub mod request;
mod tests;

use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use fancy_regex::Regex;
use indexmap::{IndexMap, IndexSet};

pub const CHAT_COMPLETIONS_PREFIX: &str = "chat_completions/";
pub const CLAUDE_PLATFORM_PREFIX: &str = "claude_platform/";
pub const CONVERSE_PREFIX: &str = "converse/";
pub const MANTLE_PREFIX: &str = "mantle/";

const APPLICATION_PROFILE_MARKER: &str = ":application-inference-profile/";
const BEDROCK_PREFIX: &str = "bedrock/";
const DEFAULT_SINCE: (u16, u16) = (5, 6);
const NOVA_PREFIXES: [&str; 2] = ["nova-2/", "nova/"];
const OPENAI_FAMILY_PATTERN: &str = r"(^|[./])openai\.";
const OPENAI_GPT_VERSION_PATTERN: &str =
    r"(^|[./])openai\.gpt-(\d{1,3})(?!\d)(?:\.(\d{1,3})(?!\d))?";
const ROUTE_TABLE: [(&str, BedrockRoute); 9] = [
    ("invoke/", BedrockRoute::Invoke),
    (CLAUDE_PLATFORM_PREFIX, BedrockRoute::ClaudePlatform),
    ("converse_like/", BedrockRoute::ConverseLike),
    (CONVERSE_PREFIX, BedrockRoute::Converse),
    ("agent/", BedrockRoute::Agent),
    ("agentcore/", BedrockRoute::Agentcore),
    ("async_invoke/", BedrockRoute::AsyncInvoke),
    ("openai/", BedrockRoute::Openai),
    (MANTLE_PREFIX, BedrockRoute::Mantle),
];
const STRIPPED_PREFIXES: [&str; 2] = [CONVERSE_PREFIX, CHAT_COMPLETIONS_PREFIX];
const WORKSPACE_KEYS: [&str; 3] = ["workspace_id", "aws_workspace_id", "anthropic_workspace_id"];
const CONVERSE_ONLY_REQUEST_KEYS: [&str; 10] = [
    "guardrailConfig",
    "performanceConfig",
    "serviceTier",
    "requestMetadata",
    "outputConfig",
    "thinking",
    "additionalModelRequestFields",
    "top_k",
    "stop",
    "model_id",
];

pub type ParamMap = IndexMap<String, ParamValue>;

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(untagged)]
pub enum ParamValue {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    Text(String),
    List(Vec<Self>),
    Mapping(IndexMap<String, Self>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BedrockRoute {
    Converse,
    Invoke,
    ClaudePlatform,
    ConverseLike,
    Agent,
    Agentcore,
    AsyncInvoke,
    Openai,
    Mantle,
    ChatCompletions,
}

#[derive(Debug, thiserror::Error)]
pub enum RouteError {
    #[error("model price map entry {key} is not an object")]
    MalformedPriceEntry { key: String },
    #[error("route pattern is invalid")]
    Pattern(#[from] fancy_regex::Error),
    #[error("request field {field} has a type the route cannot read")]
    WrongType { field: &'static str },
}

pub struct ModelRouteSelector<'catalog> {
    price_map: &'catalog ParamMap,
    converse_models: &'catalog IndexSet<String>,
    request_metadata_owned: bool,
    gpt_version_pattern: Regex,
    openai_family_pattern: Regex,
}

impl<'catalog> ModelRouteSelector<'catalog> {
    pub fn new(
        price_map: &'catalog ParamMap,
        converse_models: &'catalog IndexSet<String>,
        request_metadata_owned: bool,
    ) -> Result<Self, RouteError> {
        Ok(Self {
            price_map,
            converse_models,
            request_metadata_owned,
            gpt_version_pattern: Regex::new(OPENAI_GPT_VERSION_PATTERN)?,
            openai_family_pattern: Regex::new(OPENAI_FAMILY_PATTERN)?,
        })
    }

    pub fn route(
        &self,
        model: &str,
        request_params: Option<&ParamMap>,
    ) -> Result<BedrockRoute, RouteError> {
        if let Some(route) = named_route(model) {
            return Ok(route);
        }
        if has_route_prefix(model, CHAT_COMPLETIONS_PREFIX) {
            return self.chat_or_converse(model, request_params);
        }
        let after_bedrock = model.replacen(BEDROCK_PREFIX, "", 1);
        if NOVA_PREFIXES
            .iter()
            .any(|prefix| after_bedrock.starts_with(prefix))
        {
            return Ok(BedrockRoute::Converse);
        }
        if model.contains(APPLICATION_PROFILE_MARKER) {
            return Ok(BedrockRoute::Converse);
        }
        if catalog::chat_completions_is_default(self, model)? {
            return self.chat_or_converse(model, request_params);
        }
        let base_model = get_bedrock_base_model(model);
        let alternate_model = strip_bedrock_routing_prefix(model);
        if self.converse_models.contains(&base_model)
            || self.converse_models.contains(&alternate_model)
        {
            return Ok(BedrockRoute::Converse);
        }
        if self.openai_family_pattern.is_match(&base_model)? {
            return Ok(BedrockRoute::Converse);
        }
        Ok(BedrockRoute::Invoke)
    }

    fn chat_or_converse(
        &self,
        model: &str,
        request_params: Option<&ParamMap>,
    ) -> Result<BedrockRoute, RouteError> {
        if let Some(params) = request_params
            && request::request_needs_converse(self, model, params)?
        {
            return Ok(BedrockRoute::Converse);
        }
        Ok(BedrockRoute::ChatCompletions)
    }

    pub fn route_for_request(
        &self,
        model: &str,
        request_params: &ParamMap,
        additional_drop_params: Option<&[String]>,
    ) -> Result<BedrockRoute, RouteError> {
        let dropped: IndexSet<&str> = additional_drop_params
            .unwrap_or_default()
            .iter()
            .map(String::as_str)
            .collect();
        let filtered: ParamMap = request_params
            .iter()
            .filter(|(key, _)| !dropped.contains(key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        self.route(model, Some(&filtered))
    }
}

fn named_route(model: &str) -> Option<BedrockRoute> {
    ROUTE_TABLE
        .iter()
        .find_map(|(prefix, route)| has_route_prefix(model, prefix).then_some(*route))
}

#[must_use]
pub fn has_route_prefix(model: &str, prefix: &str) -> bool {
    let at_start = model.starts_with(prefix);
    let after_slash = format!("/{prefix}");
    model.contains(&after_slash) || at_start
}

#[must_use]
pub fn without_bedrock_route_prefix(model: &str) -> String {
    let mut stripped = model.to_owned();
    for prefix in STRIPPED_PREFIXES {
        stripped = stripped.replace(prefix, "");
    }
    stripped
}

#[must_use]
pub fn get_claude_platform_model(model: &str) -> String {
    match model.split_once(CLAUDE_PLATFORM_PREFIX) {
        Some((before, after)) => format!("{before}{after}"),
        None => model.to_owned(),
    }
}

#[must_use]
pub fn get_bedrock_tool_name<'names>(
    mappings: &'names IndexMap<String, String>,
    response_tool_name: &'names str,
) -> &'names str {
    if let Some(original) = mappings.get(response_tool_name) {
        return original;
    }
    response_tool_name
}

pub fn map_claude_platform_auth(passed_params: &ParamMap, optional_params: &mut ParamMap) {
    for key in WORKSPACE_KEYS {
        if let Some(value) = passed_params.get(key) {
            optional_params.insert(key.to_owned(), value.clone());
        }
    }
}
