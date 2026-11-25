use async_trait::async_trait;
use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::usage_sum;
use betterlite_openai::api::responses_shapes::websocket::{
    PresidioGuardrailCallback, ResponsesClientWebSocket,
};
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::ResponsesWsRelay;
use super::backend::NativeWsBackend;
use crate::api::response_id_traversal::metadata::AffinityMetadata;
use crate::api::response_id_traversal::response_id::{rewrite_value, transcode};
use crate::api::responses_stream_primitives::error_fields::{
    StreamErrorAttributes, StreamErrorObject,
};
use crate::api::responses_stream_primitives::failure::map_stream_error_to_exception;
use crate::api::responses_ws_frame_policy::events::{wrap_item_encrypted_content, ws_event_error};
use crate::api::responses_ws_frame_policy::{
    FAILURE_EVENT_TYPES, LOGGED_EVENT_TYPES, OUTPUT_ITEM_EVENT_TYPES,
};
use crate::api::responses_ws_native::error::WsNativeError;
use crate::api::responses_ws_native::logging::{WsNativeLogging, traceback_text};
use crate::api::responses_ws_native::request::{is_supplied, json_object};
use crate::cost::realtime::RealtimeBilling;
use crate::cost::realtime::error::RealtimeCostError;
use crate::cost::realtime::responses_ws::{collect_usage, partition_by_service_tier};

// multiple_inherent_impl leaves relay.rs the type's one inherent impl, so storage and logging are a trait
#[async_trait]
pub trait CaptureRelay {
    async fn store_event(&self, message: &str) -> Result<(), WsNativeError>;

    async fn store_input(&self, message: &str);

    async fn failure_exception(&self) -> Option<ErrorClass>;

    fn wrap_response_event(&self, response_str: &str) -> Result<String, WsNativeError>;

    async fn log_messages(&self) -> Result<(), RealtimeCostError>;
}

#[async_trait]
impl<Guard, Client, Backend> CaptureRelay for ResponsesWsRelay<'_, Guard, Client, Backend>
where
    Guard: PresidioGuardrailCallback + Sync,
    Client: ResponsesClientWebSocket + Sync,
    Backend: NativeWsBackend,
{
    async fn store_event(&self, message: &str) -> Result<(), WsNativeError> {
        let Some(parsed) = json_object(message)? else {
            return Ok(());
        };
        let Some(Json::Text(kind)) = parsed.get("type") else {
            return Ok(());
        };
        if !LOGGED_EVENT_TYPES.contains(&kind.as_str()) {
            return Ok(());
        }
        self.messages.lock().await.push(parsed);
        Ok(())
    }

    async fn store_input(&self, message: &str) {
        let collected = collect_input(message);
        self.input_messages.lock().await.extend(collected);
        if let Some(logging) = self.setup.logging {
            logging.pre_call(message, "");
        }
    }

    async fn failure_exception(&self) -> Option<ErrorClass> {
        let messages = self.messages.lock().await;
        let failed = messages
            .iter()
            .find(|event| {
                matches!(
                    event.get("type"),
                    Some(Json::Text(kind)) if FAILURE_EVENT_TYPES.contains(&kind.as_str())
                )
            })
            .cloned()?;
        drop(messages);
        let object = match ws_event_error(&failed) {
            Some(Json::Object(fields)) => StreamErrorObject::Mapping(fields),
            Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
            | None => StreamErrorObject::Attributes(StreamErrorAttributes::default()),
        };
        Some(map_stream_error_to_exception(
            object,
            self.setup.authorized_model.unwrap_or(""),
            self.setup.custom_llm_provider.unwrap_or(""),
            self.setup.redactor,
        ))
    }

    fn wrap_response_event(&self, response_str: &str) -> Result<String, WsNativeError> {
        let Some(mut event) = json_object(response_str)? else {
            return Ok(response_str.to_owned());
        };
        if let Some(Json::Object(response)) = event.get("response") {
            let mut wrapped = Json::Object(response.clone());
            let metadata =
                transcode::<Json, AffinityMetadata>(&Json::Object(self.litellm_metadata.clone()))?;
            rewrite_value(&mut wrapped, self.setup.custom_llm_provider, &metadata)?;
            event.insert(String::from("response"), wrapped);
            return Ok(serde_json::to_string(&Json::Object(event)).unwrap_or_default());
        }
        let item_event = matches!(
            event.get("type"),
            Some(Json::Text(kind)) if OUTPUT_ITEM_EVENT_TYPES.contains(&kind.as_str())
        );
        if !item_event {
            return Ok(response_str.to_owned());
        }
        wrap_item_encrypted_content(&event, &self.litellm_metadata).map_or_else(
            || Ok(response_str.to_owned()),
            |wrapped| Ok(serde_json::to_string(&Json::Object(wrapped)).unwrap_or_default()),
        )
    }

    async fn log_messages(&self) -> Result<(), RealtimeCostError> {
        let Some(logging) = self.setup.logging else {
            return Ok(());
        };
        let input_messages = self.input_messages.lock().await.clone();
        if !input_messages.is_empty() {
            logging.record_input_messages(&input_messages);
        }
        let messages = self.messages.lock().await.clone();
        if messages.is_empty() {
            return Ok(());
        }
        let Some(exception) = self.failure_exception().await else {
            logging.dispatch_success(messages, true);
            return Ok(());
        };
        record_usage_for_failure(&messages, logging)?;
        let traceback = traceback_text(&exception);
        logging.dispatch_failure(exception, traceback, true);
        Ok(())
    }
}

fn record_usage_for_failure(
    messages: &[IndexMap<String, Json>],
    logging: &dyn WsNativeLogging,
) -> Result<(), RealtimeCostError> {
    let usages = collect_usage(messages)?;
    let partition = partition_by_service_tier(messages)?;
    let service_tier = if partition.len() == 1 {
        partition.keys().next().cloned().flatten()
    } else {
        None
    };
    let billing = RealtimeBilling {
        results: messages.to_vec(),
        usage: usage_sum(&usages)?,
        service_tier,
    };
    let response_cost = logging.response_cost(&billing).unwrap_or(Decimal::ZERO);
    logging.record_partial_usage(&billing.usage, response_cost);
    Ok(())
}

fn collect_input(message: &str) -> Vec<IndexMap<String, Json>> {
    let Ok(Some(frame)) = json_object(message) else {
        return Vec::new();
    };
    if !matches!(frame.get("type"), Some(Json::Text(kind)) if kind == "response.create") {
        return Vec::new();
    }
    let mut collected = Vec::new();
    match frame.get("input") {
        Some(Json::Text(text)) => collected.push(user_message(Json::Text(text.clone()))),
        Some(Json::List(items)) => {
            for item in items {
                collect_item(item, &mut collected);
            }
        }
        None | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_)) => {}
    }
    collected
}

fn collect_item(item: &Json, collected: &mut Vec<IndexMap<String, Json>>) {
    let Json::Object(fields) = item else {
        return;
    };
    let user_message_item = matches!(fields.get("type"), Some(Json::Text(kind)) if kind == "message")
        && matches!(fields.get("role"), Some(Json::Text(role)) if role == "user");
    if !user_message_item {
        return;
    }
    match fields.get("content") {
        Some(Json::Text(text)) => collected.push(user_message(Json::Text(text.clone()))),
        Some(Json::List(blocks)) => {
            for block in blocks {
                if let Json::Object(block_fields) = block
                    && matches!(
                        block_fields.get("type"),
                        Some(Json::Text(kind)) if kind == "input_text"
                    )
                    && let Some(text) = block_fields.get("text")
                    && is_supplied(text)
                {
                    collected.push(user_message(text.clone()));
                }
            }
        }
        None | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_)) => {}
    }
}

fn user_message(content: Json) -> IndexMap<String, Json> {
    IndexMap::from([
        (String::from("role"), Json::Text(String::from("user"))),
        (String::from("content"), content),
    ])
}
