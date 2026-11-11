use betterlite_models::decisions::{Extra, Json};
use betterlite_redact::structured::SensitiveDataMasker;
use betterlite_redis::redis_connection_settings::model::settings::RedisConnectionSettings;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;

// RedisCache builds redis_kwargs from the kwargs it was constructed with; these are the resolved settings it received.
#[must_use]
pub fn redis_kwargs(settings: &RedisConnectionSettings) -> Extra {
    let masker = SensitiveDataMasker::default();
    let mut fields = Extra::new();
    text_field(&mut fields, "host", settings.host.as_ref());
    number_field(&mut fields, "port", settings.port);
    number_field(&mut fields, "db", settings.db);
    text_field(&mut fields, "username", settings.username.as_ref());
    secret_field(&mut fields, "password", settings.password.as_ref(), &masker);
    bool_field(&mut fields, "ssl", settings.ssl);
    float_field(&mut fields, "socket_timeout", settings.socket_timeout);
    float_field(
        &mut fields,
        "socket_connect_timeout",
        settings.socket_connect_timeout,
    );
    bool_field(&mut fields, "socket_keepalive", settings.socket_keepalive);
    number_field(&mut fields, "max_connections", settings.max_connections);
    number_field(
        &mut fields,
        "health_check_interval",
        settings.health_check_interval,
    );
    secret_field(
        &mut fields,
        "sentinel_password",
        settings.sentinel_password.as_ref(),
        &masker,
    );
    text_field(&mut fields, "service_name", settings.service_name.as_ref());
    secret_field(
        &mut fields,
        "ssl_password",
        settings.ssl_password.as_ref(),
        &masker,
    );
    fields
}

fn text_field(fields: &mut Extra, name: &str, value: Option<&String>) {
    if let Some(text) = value {
        fields.insert(name.to_owned(), Json::Text(text.clone()));
    }
}

fn number_field(fields: &mut Extra, name: &str, value: Option<i64>) {
    if let Some(number) = value {
        fields.insert(name.to_owned(), Json::Number(Number::from(number)));
    }
}

fn float_field(fields: &mut Extra, name: &str, value: Option<f64>) {
    if let Some(number) = value.and_then(Number::from_f64) {
        fields.insert(name.to_owned(), Json::Number(number));
    }
}

fn bool_field(fields: &mut Extra, name: &str, value: Option<bool>) {
    if let Some(flag) = value {
        fields.insert(name.to_owned(), Json::Bool(flag));
    }
}

fn secret_field(
    fields: &mut Extra,
    name: &str,
    value: Option<&SecretString>,
    masker: &SensitiveDataMasker,
) {
    if let Some(secret) = value {
        fields.insert(
            name.to_owned(),
            Json::Text(masker.mask_value(secret.expose_secret())),
        );
    }
}

mod tests;
