use crate::safe_proxy_config::ConfigValue;
use crate::safe_proxy_config::credential::is_credential_key;
use crate::safe_proxy_config::known_values::KNOWN_VALUES;
use crate::safe_proxy_config::tables::listed;

pub(crate) fn render_value(key: &str, value: &ConfigValue) -> Option<String> {
    match value {
        ConfigValue::Bool(flag) => Some(if *flag {
            String::from("true")
        } else {
            String::from("false")
        }),
        ConfigValue::Text(text) => render_text(key, text),
        ConfigValue::List(items) => render_list(key, items),
        ConfigValue::Null | ConfigValue::Number(_) | ConfigValue::Object(_) => None,
    }
}

fn render_text(key: &str, text: &str) -> Option<String> {
    if is_credential_key(key) || !listed(KNOWN_VALUES, text) {
        return None;
    }
    Some(text.to_owned())
}

fn render_list(key: &str, items: &[ConfigValue]) -> Option<String> {
    let rendered: Vec<String> = items
        .iter()
        .filter_map(|item| render_value(key, item))
        .collect();
    if rendered.is_empty() {
        return None;
    }
    Some(format!("[{}]", rendered.join(", ")))
}
