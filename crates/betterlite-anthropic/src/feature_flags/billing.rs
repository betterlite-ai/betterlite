use betterlite_prompts::arguments::json::Json;

pub const BILLING_HEADER_PREFIX: &str = "x-anthropic-billing-header:";
pub const CLAUDE_CLI_PREFIX: &str = "claude-cli/";
pub const CLAUDE_CODE_PREFIX: &str = "claude-code/";
const SUBAGENT_FIELD: &str = "cc_is_subagent";
const SUBAGENT_VALUE: &str = "true";

#[must_use]
pub fn billing_fields(text: &str) -> Option<Vec<(String, String)>> {
    let stripped = text.trim();
    if stripped.contains(['\n', '\r']) {
        return None;
    }
    let remainder = stripped.strip_prefix(BILLING_HEADER_PREFIX)?;
    let mut parsed = Vec::new();
    for raw_field in remainder.split(';') {
        let field = raw_field.trim();
        if field.is_empty() {
            continue;
        }
        let (raw_key, raw_value) = field.split_once('=')?;
        let (key, value) = (raw_key.trim(), raw_value.trim());
        if key.is_empty() || value.is_empty() {
            return None;
        }
        parsed.push((key.to_owned(), value.to_owned()));
    }
    if parsed.is_empty() {
        return None;
    }
    Some(parsed)
}

#[must_use]
pub fn billing_texts(system: &Json) -> Option<Vec<String>> {
    if let Json::Text(text) = system {
        return Some(vec![text.clone()]);
    }
    let Json::List(blocks) = system else {
        return None;
    };
    let mut texts = Vec::with_capacity(blocks.len());
    for block in blocks {
        let fields = block.as_object()?;
        if fields.get("type").and_then(Json::as_str) != Some("text") {
            return None;
        }
        let Json::Text(text) = fields.get("text")? else {
            return None;
        };
        texts.push(text.clone());
    }
    let meaningful: Vec<String> = texts
        .into_iter()
        .filter(|text| !text.trim().is_empty())
        .collect();
    (!meaningful.is_empty()).then_some(meaningful)
}

#[must_use]
pub fn is_subagent_billing_system(system: &Json) -> bool {
    let Some(texts) = billing_texts(system) else {
        return false;
    };
    let mut subagents = Vec::new();
    for text in &texts {
        let Some(fields) = billing_fields(text) else {
            return false;
        };
        subagents.extend(
            fields
                .into_iter()
                .filter(|(key, _)| key == SUBAGENT_FIELD)
                .map(|(_, value)| value),
        );
    }
    subagents.len() == 1
        && subagents
            .first()
            .is_some_and(|value| value == SUBAGENT_VALUE)
}

#[must_use]
pub fn is_one_shot_subagent_request(
    messages: &[Json],
    system: &Json,
    tools: &[Json],
    user_agent: Option<&str>,
) -> bool {
    let [message] = messages else {
        return false;
    };
    let Some(fields) = message.as_object() else {
        return false;
    };
    if fields.get("role").and_then(Json::as_str) != Some("user") {
        return false;
    }
    if !tools.is_empty() {
        return false;
    }
    let Some(agent) = user_agent else {
        return false;
    };
    (agent.starts_with(CLAUDE_CLI_PREFIX) || agent.starts_with(CLAUDE_CODE_PREFIX))
        && is_subagent_billing_system(system)
}
