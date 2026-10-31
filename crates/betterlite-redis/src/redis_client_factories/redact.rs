mod tests;

use secrecy::ExposeSecret as _;

use betterlite_redact::structured::SensitiveDataMasker;

use crate::redis_connection_settings::model::scalars::PassthroughValue;
use crate::redis_connection_settings::model::settings::RedisConnectionSettings;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigRow {
    pub key: String,
    pub value: String,
}

// litellm._redis._redis_kwargs_for_logging + SensitiveDataMasker.mask_dict, reproduced via is_sensitive_key/mask_value since mask_dict needs a disallowed Map type.
#[must_use]
pub fn redacted_config_rows(settings: &RedisConnectionSettings) -> Vec<ConfigRow> {
    let masker = SensitiveDataMasker::default();
    raw_entries(settings)
        .into_iter()
        .filter_map(|(key, value)| Some((key, value?)))
        .map(|(key, text)| ConfigRow {
            value: mask_if_sensitive(&masker, &key, text),
            key,
        })
        .collect()
}

#[must_use]
pub fn connection_type_label(settings: &RedisConnectionSettings) -> &'static str {
    let has_startup_nodes = settings
        .startup_nodes
        .as_deref()
        .is_some_and(|slice| !slice.is_empty());
    let has_sentinel_nodes = settings
        .sentinel_nodes
        .as_deref()
        .is_some_and(|slice| !slice.is_empty());
    if has_startup_nodes {
        "Redis Cluster"
    } else if has_sentinel_nodes {
        "Redis Sentinel"
    } else if settings.url.is_some() {
        "Redis (URL-based)"
    } else {
        "Standard Redis"
    }
}

fn mask_if_sensitive(masker: &SensitiveDataMasker, key: &str, text: String) -> String {
    if masker.is_sensitive_key(key, &[]) {
        masker.mask_value(&text)
    } else {
        text
    }
}

fn raw_entries(settings: &RedisConnectionSettings) -> Vec<(String, Option<String>)> {
    let mut entries = Vec::new();
    push_scalar_entries(settings, &mut entries);
    push_secret_entries(settings, &mut entries);
    push_node_entries(settings, &mut entries);
    entries.extend(passthrough_entries(settings));
    entries
}

fn push_scalar_entries(
    settings: &RedisConnectionSettings,
    entries: &mut Vec<(String, Option<String>)>,
) {
    entries.push(("host".to_owned(), settings.host.clone()));
    entries.push((
        "port".to_owned(),
        settings.port.map(|port| port.to_string()),
    ));
    entries.push(("db".to_owned(), settings.db.map(|db| db.to_string())));
    entries.push(("username".to_owned(), settings.username.clone()));
    entries.push(("ssl".to_owned(), settings.ssl.map(|flag| flag.to_string())));
    entries.push((
        "socket_timeout".to_owned(),
        settings.socket_timeout.map(|timeout| timeout.to_string()),
    ));
    entries.push((
        "socket_connect_timeout".to_owned(),
        settings
            .socket_connect_timeout
            .map(|timeout| timeout.to_string()),
    ));
    entries.push((
        "socket_keepalive".to_owned(),
        settings.socket_keepalive.map(|flag| flag.to_string()),
    ));
    entries.push((
        "max_connections".to_owned(),
        settings.max_connections.map(|limit| limit.to_string()),
    ));
    entries.push((
        "health_check_interval".to_owned(),
        settings
            .health_check_interval
            .map(|interval| interval.to_string()),
    ));
}

fn push_secret_entries(
    settings: &RedisConnectionSettings,
    entries: &mut Vec<(String, Option<String>)>,
) {
    entries.push((
        "password".to_owned(),
        settings
            .password
            .as_ref()
            .map(|value| value.expose_secret().to_owned()),
    ));
    entries.push((
        "url".to_owned(),
        settings
            .url
            .as_ref()
            .map(|value| value.expose_secret().to_owned()),
    ));
    entries.push((
        "credential_provider".to_owned(),
        settings
            .credential_provider_settings
            .as_ref()
            .map(|_| "<credential provider>".to_owned()),
    ));
    entries.push((
        "sentinel_password".to_owned(),
        settings
            .sentinel_password
            .as_ref()
            .map(|value| value.expose_secret().to_owned()),
    ));
    entries.push(("service_name".to_owned(), settings.service_name.clone()));
    entries.push((
        "ssl_password".to_owned(),
        settings
            .ssl_password
            .as_ref()
            .map(|value| value.expose_secret().to_owned()),
    ));
}

fn push_node_entries(
    settings: &RedisConnectionSettings,
    entries: &mut Vec<(String, Option<String>)>,
) {
    entries.push((
        "startup_nodes".to_owned(),
        node_count_text(settings.startup_nodes.as_deref(), "cluster"),
    ));
    entries.push((
        "sentinel_nodes".to_owned(),
        node_count_text(settings.sentinel_nodes.as_deref(), "sentinel"),
    ));
}

fn passthrough_entries(settings: &RedisConnectionSettings) -> Vec<(String, Option<String>)> {
    let mut entries = Vec::new();
    if settings.passthrough.contains_key("redis_connect_func") {
        entries.push((
            "redis_connect_func".to_owned(),
            Some("<redis connect function>".to_owned()),
        ));
    }
    entries.extend(
        settings
            .passthrough
            .iter()
            .filter(|(key, _)| key.as_str() != "redis_connect_func")
            .map(|(key, raw)| (key.clone(), Some(stringify_passthrough(raw)))),
    );
    entries
}

fn node_count_text<T>(nodes: Option<&[T]>, label: &str) -> Option<String> {
    match nodes {
        Some([]) => Some("[]".to_owned()),
        Some(slice) => Some(format!("[{} {label} nodes]", slice.len())),
        None => None,
    }
}

fn stringify_passthrough(raw: &PassthroughValue) -> String {
    match raw {
        PassthroughValue::Bool(flag) => flag.to_string(),
        PassthroughValue::Number(number) => number.to_string(),
        PassthroughValue::Text(text) => text.clone(),
        PassthroughValue::List(items) => format!("[{} items]", items.len()),
        PassthroughValue::Map(fields) => format!("{{{} fields}}", fields.len()),
    }
}
