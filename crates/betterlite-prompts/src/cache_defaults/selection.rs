use crate::arguments::json::{Json, Object};
use crate::cache_accounting::census::{count_external_breakpoints, count_route_breakpoints};
use crate::cache_accounting::control::default_control;
use crate::cache_accounting::extraction::request_user_agent;
use crate::cache_accounting::points::{
    CacheControlInjectionPoint, InjectionIndex, InjectionRole, MessageInjectionPoint,
};
use crate::cache_placement::marks::count_request_marks;
use crate::cache_placement::system::SystemPrompt;
use crate::message_content::Message;

use super::{CacheFacts, CachePolicy};

// LiteLLM honors automatic caching only for Claude models on a transport that carries explicit marks.
const ANTHROPIC_CACHE_PROVIDERS: [&str; 4] = ["anthropic", "bedrock", "vertex_ai", "azure_ai"];
const CLAUDE_CODE_USER_AGENTS: [&str; 2] = ["claude-cli/", "claude-code/"];
const BILLING_HEADER: &str = "x-anthropic-billing-header:";
const SUBAGENT_FIELD: &str = "cc_is_subagent";
const SUBAGENT_VALUE: &str = "true";
const REQUEST_VIEW_KEYS: [&str; 2] = ["extra_body", "proxy_server_request"];

pub struct DefaultRequest<'input> {
    pub model: &'input str,
    pub custom_llm_provider: Option<&'input str>,
    pub messages: &'input [Message],
    pub system: Option<&'input SystemPrompt>,
    pub tools: Option<&'input [Json]>,
    pub enable_prompt_caching: Option<bool>,
    pub cache_control: Option<&'input Json>,
    pub request_kwargs: &'input Object,
    pub on_messages_route: bool,
    pub policy: CachePolicy<'input>,
}

// The two breakpoints LiteLLM synthesizes: the stable system prefix and the trailing turn.
#[must_use]
pub fn get_default_injection_points(
    request: &DefaultRequest<'_>,
    facts: &dyn CacheFacts,
) -> Vec<CacheControlInjectionPoint> {
    if !request.policy.global_enabled && request.enable_prompt_caching != Some(true) {
        return Vec::new();
    }
    if !anthropic_cache_support(request.model, request.custom_llm_provider, facts) {
        return Vec::new();
    }
    let wire = request_view(request.request_kwargs);
    if request_has_cache_control(request, &wire) {
        return Vec::new();
    }
    let user_agent = request_user_agent(&wire);
    if one_shot_subagent_request(request, user_agent) {
        return Vec::new();
    }
    let control = default_control(request.policy.ttl);
    vec![
        default_point(Some(InjectionRole::System), None, &control),
        default_point(None, Some(InjectionIndex::Number(-1)), &control),
    ]
}

fn default_point(
    role: Option<InjectionRole>,
    index: Option<InjectionIndex>,
    control: &Json,
) -> CacheControlInjectionPoint {
    CacheControlInjectionPoint::Message(MessageInjectionPoint {
        role,
        index,
        control: Some(control.clone()),
        openai_dialect: None,
        external_breakpoints: None,
    })
}

// LiteLLM swallows an unroutable model here rather than failing the optional caching feature.
fn anthropic_cache_support(
    model: &str,
    custom_llm_provider: Option<&str>,
    facts: &dyn CacheFacts,
) -> bool {
    let provider = match custom_llm_provider {
        Some(name) => name.to_owned(),
        None => match facts.provider(model) {
            Some(name) => name,
            None => return false,
        },
    };
    ANTHROPIC_CACHE_PROVIDERS.contains(&provider.as_str())
        && model.to_lowercase().contains("claude")
        && facts.supports_prompt_caching(model, &provider)
}

// Only the automatic defaults stand down on a client mark; configured points apply alongside it.
fn request_has_cache_control(request: &DefaultRequest<'_>, wire: &Json) -> bool {
    let external = if request.on_messages_route {
        count_route_breakpoints(request.tools, request.cache_control, Some(wire))
    } else {
        count_external_breakpoints(request.tools, request.cache_control, Some(wire))
    };
    count_request_marks(request.messages, request.system).saturating_add(external) > 0
}

// The gates read only the envelope and the proxy headers, so the view copies just those keys.
pub(super) fn request_view(kwargs: &Object) -> Json {
    let mut view = Object::new();
    for key in REQUEST_VIEW_KEYS {
        if let Some(value) = kwargs.get(key) {
            view.insert(key.to_owned(), value.clone());
        }
    }
    Json::Object(view)
}

fn one_shot_subagent_request(request: &DefaultRequest<'_>, user_agent: Option<&str>) -> bool {
    let Some(agent) = user_agent else {
        return false;
    };
    if !CLAUDE_CODE_USER_AGENTS
        .iter()
        .any(|prefix| agent.starts_with(prefix))
    {
        return false;
    }
    if request.tools.is_some_and(|tools| !tools.is_empty()) {
        return false;
    }
    let [only_message] = request.messages else {
        return false;
    };
    only_message.role == "user" && subagent_billing_system(request.system)
}

// The subagent marker rides in the billing header of the single system text block Claude Code sends.
fn subagent_billing_system(system: Option<&SystemPrompt>) -> bool {
    let Some(texts) = billing_texts(system) else {
        return false;
    };
    let mut subagent_values = Vec::new();
    for text in texts {
        let Some(fields) = billing_fields(text) else {
            return false;
        };
        for (key, value) in fields {
            if key == SUBAGENT_FIELD {
                subagent_values.push(value);
            }
        }
    }
    subagent_values == [SUBAGENT_VALUE]
}

fn billing_texts(system: Option<&SystemPrompt>) -> Option<Vec<&str>> {
    match system {
        Some(SystemPrompt::Text(text)) => Some(vec![text.as_str()]),
        Some(SystemPrompt::Blocks(blocks)) => {
            let mut texts = Vec::with_capacity(blocks.len());
            for block in blocks {
                if block.get("type").and_then(Json::as_str) != Some("text") {
                    return None;
                }
                let text = block.get("text").and_then(Json::as_str)?;
                if !text.trim().is_empty() {
                    texts.push(text);
                }
            }
            (!texts.is_empty()).then_some(texts)
        }
        None => None,
    }
}

fn billing_fields(text: &str) -> Option<Vec<(&str, &str)>> {
    let stripped = text.trim();
    if stripped.contains('\n') || stripped.contains('\r') {
        return None;
    }
    let rest = stripped.strip_prefix(BILLING_HEADER)?;
    let mut fields = Vec::new();
    for raw in rest.split(';') {
        let field = raw.trim();
        if field.is_empty() {
            continue;
        }
        let (raw_key, raw_value) = field.split_once('=')?;
        let key = raw_key.trim();
        let value = raw_value.trim();
        if key.is_empty() || value.is_empty() {
            return None;
        }
        fields.push((key, value));
    }
    (!fields.is_empty()).then_some(fields)
}
