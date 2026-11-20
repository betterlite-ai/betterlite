use std::error::Error;

use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_models::decisions::Json;
use betterlite_openai::realtime::normalizer::RealtimeEventNormalizer;
use indexmap::IndexMap;

use crate::api::realtime_beta_protocol::error::BetaProtocolError;
use crate::api::realtime_beta_protocol::session::remap_beta_session_to_ga;
use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_guardrail_policy::{TRANSCRIPTION_EVENT_HOOKS, guardrails_run_for_hooks};

use super::RealtimeClientRelay;
use super::error::RealtimeClientRelayError;
use super::rewrite::ClientEvent;

fn hook_runs<Config, CallbackError>(
    relay: &RealtimeClientRelay<'_, Config, CallbackError>,
) -> Result<bool, RealtimeClientRelayError<Config::Error, CallbackError>>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    guardrails_run_for_hooks(
        relay.callbacks.as_slice(),
        &relay.state.request_data,
        &TRANSCRIPTION_EVENT_HOOKS,
    )
    .map_err(RealtimeClientRelayError::Predicate)
}

// LiteLLM rewrites the session in place: inject once, force the flag false, remap for GA, then let the provider patch it
pub(crate) fn session_update<Config, CallbackError>(
    relay: &mut RealtimeClientRelay<'_, Config, CallbackError>,
    event: &mut ClientEvent,
) -> Result<bool, RealtimeClientRelayError<Config::Error, CallbackError>>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    let first_gate = relay.state.response.session_configuration_request.is_none()
        && !relay.state.setup.turn_detection_update_sent.0;
    let injected = if first_gate && hook_runs(relay)? {
        let applied = mutate(event, inject_first_update);
        if applied {
            tracing::debug!(
                "Injected turn_detection into first session.update for audio transcription guardrails"
            );
        }
        applied
    } else {
        false
    };
    let mut changed = injected;
    if !injected && hook_runs(relay)? {
        changed |= mutate(event, force_false_vad);
    }
    if !relay.state.backend_uses_beta_protocol.0 {
        let remapped = match &mut event.value {
            Json::Object(root) => remap_session(root).map_err(RealtimeClientRelayError::Remap)?,
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => false,
        };
        changed |= remapped;
    }
    if let Some(normalizer) = relay.state.event_normalizer.as_deref() {
        changed |= mutate(event, |root| patch_session(normalizer, root));
    }
    if changed {
        event.refresh().map_err(RealtimeClientRelayError::Encode)?;
    }
    Ok(injected)
}

fn mutate(
    event: &mut ClientEvent,
    apply: impl FnOnce(&mut IndexMap<String, Json>) -> bool,
) -> bool {
    match &mut event.value {
        Json::Object(root) => apply(root),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => false,
    }
}

fn inject_first_update(root: &mut IndexMap<String, Json>) -> bool {
    if !root.contains_key("session") {
        root.insert("session".to_owned(), Json::Object(IndexMap::new()));
    }
    let Some(Json::Object(session)) = root.get_mut("session") else {
        return false;
    };
    if !matches!(session.get("turn_detection"), Some(Json::Object(_))) {
        session.insert("turn_detection".to_owned(), Json::Object(IndexMap::new()));
    }
    let Some(Json::Object(turn_detection)) = session.get_mut("turn_detection") else {
        return false;
    };
    turn_detection.insert("create_response".to_owned(), Json::Bool(false));
    true
}

fn force_false_vad(root: &mut IndexMap<String, Json>) -> bool {
    let Some(Json::Object(session)) = root.get_mut("session") else {
        return false;
    };
    let mut overridden = false;
    let flat_present = session
        .get("turn_detection")
        .is_some_and(|value| !matches!(value, Json::Null));
    if flat_present {
        if !matches!(session.get("turn_detection"), Some(Json::Object(_))) {
            session.insert("turn_detection".to_owned(), Json::Object(IndexMap::new()));
        }
        if let Some(Json::Object(flat)) = session.get_mut("turn_detection")
            && flat.get("create_response") != Some(&Json::Bool(false))
        {
            flat.insert("create_response".to_owned(), Json::Bool(false));
            overridden = true;
        }
    }
    let mut nested_present = false;
    if let Some(Json::Object(audio)) = session.get_mut("audio")
        && let Some(Json::Object(input)) = audio.get_mut("input")
    {
        nested_present = input
            .get("turn_detection")
            .is_some_and(|value| !matches!(value, Json::Null));
        if nested_present {
            if !matches!(input.get("turn_detection"), Some(Json::Object(_))) {
                input.insert("turn_detection".to_owned(), Json::Object(IndexMap::new()));
            }
            if let Some(Json::Object(nested)) = input.get_mut("turn_detection")
                && nested.get("create_response") != Some(&Json::Bool(false))
            {
                nested.insert("create_response".to_owned(), Json::Bool(false));
                overridden = true;
            }
        }
    }
    if !flat_present && !nested_present {
        let mut turn_detection = IndexMap::new();
        turn_detection.insert("create_response".to_owned(), Json::Bool(false));
        session.insert("turn_detection".to_owned(), Json::Object(turn_detection));
        overridden = true;
    }
    overridden
}

fn remap_session(root: &mut IndexMap<String, Json>) -> Result<bool, BetaProtocolError> {
    let source = match root.get("session") {
        None => Json::Object(IndexMap::new()),
        Some(value @ Json::Object(_)) => value.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Ok(false);
        }
    };
    let remapped = remap_beta_session_to_ga(source)?;
    root.insert("session".to_owned(), remapped);
    Ok(true)
}

fn patch_session(
    normalizer: &dyn RealtimeEventNormalizer,
    root: &mut IndexMap<String, Json>,
) -> bool {
    let Some(session @ Json::Object(_)) = root.get("session") else {
        return false;
    };
    let patched = normalizer.patch_outgoing_session(session.clone());
    root.insert("session".to_owned(), patched);
    true
}
