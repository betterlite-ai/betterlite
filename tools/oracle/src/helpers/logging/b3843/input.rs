use betterlite_logging::otel::model::payloads::proxy_request::ProxyRequestSpanData;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

type Fields = BTreeMap<String, OtelModelValue>;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    RequestParams { value: Fields },
    Cost { value: Fields },
    Usage { value: Fields },
    ServerInfo { value: Option<String> },
    Service { value: Value },
    SpanError { value: Fields },
    ProxyRequest { value: Box<ProxyRequestSpanData> },
    Guardrail { entry: Fields },
    GuardrailErrorStatus { entry: Fields },
    CacheTokenValue { values: Vec<OtelModelValue> },
    GuardrailModeStr { value: OtelModelValue },
    TotalMaskedEntities { value: OtelModelValue },
    Dicts { value: OtelModelValue },
    JsonOrNone { value: OtelModelValue },
    SanitizeEventMetadata { value: Fields },
}
