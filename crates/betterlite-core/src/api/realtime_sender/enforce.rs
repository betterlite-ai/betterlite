use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::RealtimeSender;
use super::error::RealtimeSenderError;
use crate::api::realtime_contracts::config::BaseRealtimeConfig;

impl<Config, CallbackError> RealtimeSender<'_, Config, CallbackError>
where
    Config: BaseRealtimeConfig,
    CallbackError: std::error::Error + 'static,
{
    // LiteLLM rewrites every transcription model a session update carries to the model the proxy already authorized
    pub fn enforce_session_model(
        &mut self,
        message: &str,
    ) -> Result<String, RealtimeSenderError<Config::Error, CallbackError>> {
        let Some(authorized_model) = self.state.force_transcription_model.clone() else {
            return Ok(message.to_owned());
        };
        let Ok(mut decoded) = serde_json::from_str::<Json>(message) else {
            return Ok(message.to_owned());
        };
        let Json::Object(root) = &mut decoded else {
            return Err(RealtimeSenderError::SessionUpdateNotObject);
        };
        if !is_session_update(root) {
            return Ok(message.to_owned());
        }
        let Some(Json::Object(session)) = root.get_mut("session") else {
            return Ok(message.to_owned());
        };
        if matches!(session.get("type"), Some(Json::Text(kind)) if kind == "transcription") {
            self.state.is_transcription_session.0 = true;
        }
        if !rewrite_models(session, &authorized_model) {
            return Ok(message.to_owned());
        }
        serde_json::to_string(&decoded).map_err(|source| RealtimeSenderError::Encode { source })
    }
}

fn is_session_update(root: &IndexMap<String, Json>) -> bool {
    let kind = match root.get("type") {
        Some(Json::Text(kind)) => kind.as_str(),
        _ => return false,
    };
    kind == "session.update" || kind == "transcription_session.update"
}

fn rewrite_models(session: &mut IndexMap<String, Json>, authorized_model: &str) -> bool {
    let mut changed = false;
    if let Some(Json::Object(transcription)) = session.get_mut("input_audio_transcription") {
        if set_model(transcription, authorized_model) {
            changed = true;
        }
    }
    if let Some(transcription) = nested_transcription(session) {
        if set_model(transcription, authorized_model) {
            changed = true;
        }
    }
    changed
}

// The GA shape nests the transcription object under audio.input; LiteLLM rebuilds the same dictionaries in place
fn nested_transcription(
    session: &mut IndexMap<String, Json>,
) -> Option<&mut IndexMap<String, Json>> {
    let Json::Object(audio) = session.get_mut("audio")? else {
        return None;
    };
    let Json::Object(input) = audio.get_mut("input")? else {
        return None;
    };
    let Json::Object(transcription) = input.get_mut("transcription")? else {
        return None;
    };
    Some(transcription)
}

fn set_model(transcription: &mut IndexMap<String, Json>, authorized_model: &str) -> bool {
    let unchanged = matches!(
        transcription.get("model"),
        Some(Json::Text(model)) if model == authorized_model
    );
    if unchanged {
        return false;
    }
    transcription.insert("model".to_owned(), Json::Text(authorized_model.to_owned()));
    true
}
