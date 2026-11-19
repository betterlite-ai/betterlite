use betterlite_core::api::responses_stream_driver::errors::{DriverError, TransportKind};
use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use serde_json::Value;

use super::super::assemble::Event;
use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::result::{Failure, Nullable};

pub(super) fn view(event: &Event) -> Result<Json, SdkHttpError> {
    let value = serde_json::to_value(event).map_err(SdkHttpError::Serialize)?;
    let typed = serde_json::from_value(value).map_err(SdkHttpError::ResultParse)?;
    Ok(drop_nulls(typed))
}

fn drop_nulls(value: Json) -> Json {
    match value {
        Json::Object(fields) => Json::Object(
            fields
                .into_iter()
                .filter_map(|(key, child)| match child {
                    Json::Null => None,
                    other => Some((key, drop_nulls(other))),
                })
                .collect(),
        ),
        Json::List(items) => Json::List(items.into_iter().map(drop_nulls).collect()),
        other => other,
    }
}

pub(super) fn failure_of(error: &DriverError) -> Result<Failure, SdkHttpError> {
    match error {
        DriverError::Class { class } => {
            let metadata = class.metadata();
            Ok(Failure {
                class: class_name(class)?.to_owned(),
                status: metadata.status_code,
                kind: wire(&metadata.error_type),
                param: wire(&metadata.param),
                code: Some(metadata.openai_code),
                message: message_of(class),
            })
        }
        DriverError::Transport(failure) => Ok(Failure {
            class: transport_class(failure.kind).to_owned(),
            status: 0,
            kind: None,
            param: None,
            code: None,
            message: failure.message.clone(),
        }),
    }
}

fn wire(field: &WireField) -> Nullable {
    match field {
        WireField::Missing(_) => None,
        WireField::Null => Some(None),
        WireField::Text(text) => Some(Some(text.clone())),
    }
}

// native::class_name stays private to its module, so the classes this driver raises are named here
fn class_name(class: &ErrorClass) -> Result<&'static str, SdkHttpError> {
    match class {
        ErrorClass::MidStreamFallback(_) => Ok("MidStreamFallbackError"),
        ErrorClass::ApiConnection(_) => Ok("APIConnectionError"),
        ErrorClass::Timeout(_) => Ok("Timeout"),
        ErrorClass::RateLimit(_) => Ok("RateLimitError"),
        ErrorClass::BadRequest(_) => Ok("BadRequestError"),
        other => Err(SdkHttpError::Unprojected(format!("{other:?}"))),
    }
}

// the only read failure a scripted case injects outside the transport pair is httpx.ReadTimeout
fn transport_class(kind: TransportKind) -> &'static str {
    match kind {
        TransportKind::ReadError => "ReadError",
        TransportKind::RemoteProtocolError => "RemoteProtocolError",
        TransportKind::HttpError => "ReadTimeout",
    }
}

fn message_of(class: &ErrorClass) -> String {
    let Ok(value) = serde_json::to_value(class) else {
        return String::new();
    };
    value
        .pointer("/context/message")
        .or_else(|| value.pointer("/message"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}
