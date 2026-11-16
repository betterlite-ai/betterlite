use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::event_types::Extra;
use serde_json::Number;

pub(super) fn numbered(sequence_number: i64) -> Extra {
    let mut extra = Extra::new();
    extra.insert(
        "sequence_number".to_owned(),
        Json::Number(Number::from(sequence_number)),
    );
    extra
}
