use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::meta_muse_realtime::MuseSessionConfig;
use betterlite_core::meta_muse_realtime::session::parse_session_update;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;

#[derive(Debug, Deserialize)]
pub(crate) struct SessionInput {
    payloads: Vec<String>,
    expected_model: String,
    access_token: String,
    session_id: String,
}

pub(crate) fn run(fields: &SessionInput) -> Value {
    let configs: Vec<Value> = fields
        .payloads
        .iter()
        .map(|payload| {
            parse_session_update(payload, &fields.expected_model, ApiLimits::default())
                .map_or(Value::Null, |config| document(&config, fields))
        })
        .collect();
    json!({"configs": configs})
}

fn document(config: &MuseSessionConfig, fields: &SessionInput) -> Value {
    let token = SecretString::from(fields.access_token.clone());
    json!({
        "model": config.model,
        "mode": config.mode,
        "sample_rate": config.sample_rate.hertz(),
        "language_bias": config.language_bias,
        "audio_encoding": config.sample_rate.audio_encoding(),
        "bytes_per_second": config.sample_rate.bytes_per_second(),
        "packet_bytes": config.sample_rate.packet_bytes(),
        "max_encoded_append_bytes": config.sample_rate.max_encoded_append_bytes(),
        "handshake": config.handshake(&token),
        "openai_session": config.openai_session(&fields.session_id),
    })
}
