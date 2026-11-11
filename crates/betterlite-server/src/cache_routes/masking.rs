use betterlite_models::decisions::{Extra, Json};
use betterlite_redact::structured::SensitiveDataMasker;
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use url::Url;

use self::columns::COLUMNS;
use self::kwarg_value::KwargValue as _;

mod columns;
mod kwarg_value;

// LiteLLM's RedisCache.redis_kwargs, masked: the kwargs the cache was built with, before any REDIS_* variable is read.
#[must_use]
pub fn redis_kwargs(overrides: &RedisOverrides) -> Extra {
    let mut fields = Extra::new();
    for (name, column) in COLUMNS {
        if let Some(value) = column(overrides) {
            fields.insert(name.to_owned(), value);
        }
    }
    for (name, value) in &overrides.passthrough {
        fields.insert(name.clone(), value.kwarg());
    }
    mask_object(&SensitiveDataMasker::default(), &mut fields);
    fields
}

// SensitiveDataMasker.mask_dict's walk over a typed object: mask_dict itself takes serde_json::Map, which the workspace disallows.
fn mask_object(masker: &SensitiveDataMasker, object: &mut Extra) {
    for (key, value) in object.iter_mut() {
        if key == "url" {
            redact_url(masker, value);
        }
        mask_node(masker, value, masker.is_sensitive_key(key, &[]));
    }
}

fn mask_node(masker: &SensitiveDataMasker, value: &mut Json, sensitive: bool) {
    match value {
        Json::Object(inner) => mask_object(masker, inner),
        Json::List(items) => {
            for item in items {
                match item {
                    Json::Text(text) if sensitive => *text = masker.mask_value(text),
                    Json::Object(_) | Json::List(_) => mask_node(masker, item, sensitive),
                    Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {}
                }
            }
        }
        Json::Text(text) if sensitive => *text = masker.mask_value(text),
        Json::Number(number) if sensitive => {
            *value = Json::Text(masker.mask_value(&number.to_string()));
        }
        Json::Bool(flag) if sensitive => {
            *value = Json::Text(masker.mask_value(&flag.to_string()));
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {}
    }
}

// Deviation: LiteLLM prints url as given, password included; the userinfo password and the sensitive query values are masked here.
fn redact_url(masker: &SensitiveDataMasker, value: &mut Json) {
    let Json::Text(text) = value else {
        return;
    };
    let Ok(mut parsed) = Url::parse(text) else {
        *text = masker.mask_value(text);
        return;
    };
    if let Some(password) = parsed.password().map(str::to_owned) {
        let hidden = masker.mask_value(&password);
        if parsed.set_password(Some(&hidden)).is_err() {
            *text = masker.mask_value(text);
            return;
        }
    }
    let pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .map(|(name, pair_value)| {
            let shown = if masker.is_sensitive_key(&name, &[]) {
                masker.mask_value(&pair_value)
            } else {
                pair_value.into_owned()
            };
            (name.into_owned(), shown)
        })
        .collect();
    if !pairs.is_empty() {
        parsed.query_pairs_mut().clear().extend_pairs(pairs);
    }
    *text = parsed.to_string();
}

mod tests;
