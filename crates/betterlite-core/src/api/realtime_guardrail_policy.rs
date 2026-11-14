pub mod error;

mod tests;

use std::error::Error;

use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::realtime_guardrail_policy::error::RealtimeGuardrailPolicyError;
use crate::api::request_destination::RequestData;

// One litellm.callbacks entry: LiteLLM asks isinstance(cb, CustomGuardrail) before it asks the predicate, and detection counts a native-lifecycle guardrail that B1007's runner later skips.
pub trait RealtimeGuardrailCallback {
    type Error;

    fn is_custom_guardrail(&self) -> bool;

    fn should_run_guardrail(
        &self,
        data: &RequestData,
        event: GuardrailEventHooks,
    ) -> Result<bool, Self::Error>;
}

// Only this hook turns off provider VAD auto-response; pre_call and post_call guardrails must not.
pub const TRANSCRIPTION_EVENT_HOOKS: [GuardrailEventHooks; 1] =
    [GuardrailEventHooks::RealtimeInputTranscription];

pub fn guardrails_run_for_hooks<Callback>(
    callbacks: &[&Callback],
    request_data: &RequestData,
    event_hooks: &[GuardrailEventHooks],
) -> Result<bool, Callback::Error>
where
    Callback: RealtimeGuardrailCallback + ?Sized,
{
    for callback in callbacks {
        if !callback.is_custom_guardrail() {
            continue;
        }
        for event in event_hooks {
            if callback.should_run_guardrail(request_data, *event)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

pub fn disable_auto_response_message(
    backend_uses_beta_protocol: bool,
) -> Result<String, RealtimeGuardrailPolicyError> {
    let turn_detection = Json::Object(IndexMap::from([
        ("type".to_owned(), Json::Text("server_vad".to_owned())),
        ("create_response".to_owned(), Json::Bool(false)),
    ]));
    let session = if backend_uses_beta_protocol {
        Json::Object(IndexMap::from([(
            "turn_detection".to_owned(),
            turn_detection,
        )]))
    } else {
        Json::Object(IndexMap::from([
            ("type".to_owned(), Json::Text("realtime".to_owned())),
            (
                "audio".to_owned(),
                Json::Object(IndexMap::from([(
                    "input".to_owned(),
                    Json::Object(IndexMap::from([(
                        "turn_detection".to_owned(),
                        turn_detection,
                    )])),
                )])),
            ),
        ]))
    };
    let message = Json::Object(IndexMap::from([
        ("type".to_owned(), Json::Text("session.update".to_owned())),
        ("session".to_owned(), session),
    ]));
    serde_json::to_string(&message)
        .map_err(|source| RealtimeGuardrailPolicyError::Encode { source })
}

pub fn inject_auto_response_disable<Callback>(
    setup_message: &str,
    sent: &mut bool,
    callbacks: &[&Callback],
    request_data: &RequestData,
) -> Result<Option<String>, RealtimeGuardrailPolicyError<Callback::Error>>
where
    Callback: RealtimeGuardrailCallback + ?Sized,
    Callback::Error: Error + 'static,
{
    if *sent || !guardrails_run_for_hooks(callbacks, request_data, &TRANSCRIPTION_EVENT_HOOKS)? {
        return Ok(None);
    }
    let Ok(mut message) = serde_json::from_str::<Json>(setup_message) else {
        return Ok(None);
    };
    let Json::Object(root) = &mut message else {
        return Ok(None);
    };
    let Some(Json::Object(setup)) = root.get_mut("setup") else {
        return Ok(None);
    };
    let config_value = setup
        .entry("realtimeInputConfig".to_owned())
        .or_insert_with(|| Json::Object(IndexMap::new()));
    let Json::Object(config) = config_value else {
        return Err(RealtimeGuardrailPolicyError::SetupFieldNotObject {
            field: "realtimeInputConfig",
        });
    };
    let detection_value = config
        .entry("automaticActivityDetection".to_owned())
        .or_insert_with(|| Json::Object(IndexMap::new()));
    let Json::Object(automatic) = detection_value else {
        return Err(RealtimeGuardrailPolicyError::SetupFieldNotObject {
            field: "automaticActivityDetection",
        });
    };
    automatic.insert("disabled".to_owned(), Json::Bool(true));
    *sent = true;
    tracing::debug!(
        "Realtime: folded automaticActivityDetection.disabled=true into setup for transcription-guardrail gating"
    );
    serde_json::to_string(&message)
        .map(Some)
        .map_err(|source| RealtimeGuardrailPolicyError::Encode { source })
}
