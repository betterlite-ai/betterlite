use std::collections::BTreeSet;

use crate::bug_report::providers::KNOWN_PROVIDERS;
use crate::safe_proxy_config::render::render_value;
use crate::safe_proxy_config::tables::{GUARDRAIL_KEYS, listed};
use crate::safe_proxy_config::{ConfigObject, ConfigValue};

pub(crate) fn section_lines(
    section: &str,
    values: Option<&ConfigObject>,
    known_keys: &str,
) -> Vec<String> {
    let mut lines = Vec::new();
    let Some(settings) = values else {
        return lines;
    };
    for (key, value) in settings {
        if !listed(known_keys, key) {
            continue;
        }
        if let Some(rendered) = render_value(key, value) {
            lines.push(format!("{section}.{key} = {rendered}"));
        }
    }
    lines
}

pub(crate) fn guardrail_lines(guardrails: Option<&ConfigValue>) -> Vec<String> {
    let mut lines = Vec::new();
    let Some(ConfigValue::List(entries)) = guardrails else {
        return lines;
    };
    for (index, entry) in entries.iter().enumerate() {
        let ConfigValue::Object(guardrail) = entry else {
            continue;
        };
        let section = format!("guardrails[{index}].litellm_params");
        let params = object_at(guardrail, "litellm_params");
        lines.extend(section_lines(&section, params, GUARDRAIL_KEYS));
    }
    lines
}

pub(crate) fn model_list_lines(model_list: Option<&ConfigValue>) -> Vec<String> {
    let Some(ConfigValue::List(entries)) = model_list else {
        return Vec::new();
    };
    let mut providers = BTreeSet::new();
    for entry in entries {
        let ConfigValue::Object(deployment) = entry else {
            continue;
        };
        let params = object_at(deployment, "litellm_params");
        let text = params.and_then(|settings| settings.get("model"));
        if let Some(ConfigValue::Text(model)) = text
            && let Some((provider, _)) = model.split_once('/')
            && listed(KNOWN_PROVIDERS, provider)
        {
            providers.insert(provider);
        }
    }
    if providers.is_empty() {
        return Vec::new();
    }
    let joined = providers.into_iter().collect::<Vec<&str>>().join(", ");
    vec![format!("model_list[*].provider = [{joined}]")]
}

pub(crate) fn object_at<'values>(
    values: &'values ConfigObject,
    key: &str,
) -> Option<&'values ConfigObject> {
    match values.get(key) {
        Some(ConfigValue::Object(object)) => Some(object),
        Some(
            ConfigValue::Null
            | ConfigValue::Bool(_)
            | ConfigValue::Number(_)
            | ConfigValue::Text(_)
            | ConfigValue::List(_),
        )
        | None => None,
    }
}
