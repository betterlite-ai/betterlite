use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::TRANSCRIPTION_COMPLETED;

pub(crate) fn usage_record(usage: Json) -> IndexMap<String, Json> {
    IndexMap::from([
        (
            "type".to_owned(),
            Json::Text(TRANSCRIPTION_COMPLETED.to_owned()),
        ),
        ("usage".to_owned(), usage),
    ])
}
