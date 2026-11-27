use betterlite_models::decisions::Json;
use indexmap::IndexMap;

pub const DEFAULT_REALTIME_MODEL: &str = "gpt-4o-realtime-preview";
pub const DEFAULT_TRANSCRIPTION_MODEL: &str = "gpt-realtime-whisper";

// _ALLOWED_SESSION_TYPES: exactly these two names are accepted, and every other value, absent included, is realtime
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RealtimeSessionType {
    Realtime,
    Transcription,
}

impl RealtimeSessionType {
    #[must_use]
    pub fn from_request(session_type: Option<&str>) -> Self {
        if session_type == Some("transcription") {
            Self::Transcription
        } else {
            Self::Realtime
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Realtime => "realtime",
            Self::Transcription => "transcription",
        }
    }
}

// a JSON string, or None for every other JSON value
#[must_use]
pub(super) fn text_value(value: Option<&Json>) -> Option<&str> {
    match value {
        Some(Json::Text(text)) => Some(text),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

// _append_model_candidate: only a nonempty string is a candidate, and it joins the list once, untrimmed
pub fn append_model_candidate(candidates: &mut Vec<String>, model: Option<&str>) {
    let Some(model) = model else {
        return;
    };
    if model.is_empty() || candidates.iter().any(|candidate| candidate == model) {
        return;
    }
    candidates.push(model.to_owned());
}

// _transcription_model_candidates_from_session: the GA nested model first, then the flat one
#[must_use]
pub fn model_candidates(session: &IndexMap<String, Json>) -> Vec<String> {
    let mut candidates = Vec::new();
    if let Some(Json::Object(audio)) = session.get("audio")
        && let Some(Json::Object(input)) = audio.get("input")
        && let Some(Json::Object(transcription)) = input.get("transcription")
    {
        append_model_candidate(&mut candidates, text_value(transcription.get("model")));
    }
    if let Some(Json::Object(transcription)) = session.get("input_audio_transcription") {
        append_model_candidate(&mut candidates, text_value(transcription.get("model")));
    }
    candidates
}

// _set_transcription_model_on_session: every existing transcription shape takes the model, and the GA one is created only when asked and absent
pub fn set_transcription_model(
    session: &mut IndexMap<String, Json>,
    model: &str,
    create_if_missing: bool,
) {
    let mut updated = false;
    if let Some(Json::Object(transcription)) = session.get("input_audio_transcription") {
        let mut replaced = transcription.clone();
        replaced.insert("model".to_owned(), Json::Text(model.to_owned()));
        session.insert(
            "input_audio_transcription".to_owned(),
            Json::Object(replaced),
        );
        updated = true;
    }
    if let Some(Json::Object(audio)) = session.get("audio")
        && let Some(Json::Object(input)) = audio.get("input")
        && let Some(Json::Object(transcription)) = input.get("transcription")
    {
        let mut replaced_transcription = transcription.clone();
        replaced_transcription.insert("model".to_owned(), Json::Text(model.to_owned()));
        let mut replaced_input = input.clone();
        replaced_input.insert(
            "transcription".to_owned(),
            Json::Object(replaced_transcription),
        );
        let mut replaced_audio = audio.clone();
        replaced_audio.insert("input".to_owned(), Json::Object(replaced_input));
        session.insert("audio".to_owned(), Json::Object(replaced_audio));
        updated = true;
    }
    if !updated && create_if_missing {
        create_nested_transcription(session, model);
    }
}

// the create_if_missing branch keeps whatever existing dictionary holds the audio and input, and replaces anything else
fn create_nested_transcription(session: &mut IndexMap<String, Json>, model: &str) {
    let mut audio = object_or_empty(session.get("audio"));
    let mut input = object_or_empty(audio.get("input"));
    let transcription = IndexMap::from([("model".to_owned(), Json::Text(model.to_owned()))]);
    input.insert("transcription".to_owned(), Json::Object(transcription));
    audio.insert("input".to_owned(), Json::Object(input));
    session.insert("audio".to_owned(), Json::Object(audio));
}

#[must_use]
fn object_or_empty(value: Option<&Json>) -> IndexMap<String, Json> {
    match value {
        Some(Json::Object(object)) => object.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => IndexMap::new(),
    }
}
