use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Number;

pub const ITPM_RESERVED_KEY: &str = "_litellm_itpm_reserved";
pub const OTPM_RESERVED_KEY: &str = "_litellm_otpm_reserved";
pub const ITPM_CACHE_KEY: &str = "_litellm_itpm_cache_key";
pub const OTPM_CACHE_KEY: &str = "_litellm_otpm_cache_key";

const RESERVATION_KEYS: [&str; 4] = [
    ITPM_RESERVED_KEY,
    OTPM_RESERVED_KEY,
    ITPM_CACHE_KEY,
    OTPM_CACHE_KEY,
];

type Metadata = IndexMap<String, Json>;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IoReservation {
    pub itpm_reserved: i64,
    pub otpm_reserved: i64,
    pub itpm_key: Option<String>,
    pub otpm_key: Option<String>,
}

impl IoReservation {
    fn entries(&self) -> [(&'static str, Json); 4] {
        [
            (
                ITPM_RESERVED_KEY,
                Json::Number(Number::from(self.itpm_reserved)),
            ),
            (
                OTPM_RESERVED_KEY,
                Json::Number(Number::from(self.otpm_reserved)),
            ),
            (ITPM_CACHE_KEY, text_or_null(self.itpm_key.as_ref())),
            (OTPM_CACHE_KEY, text_or_null(self.otpm_key.as_ref())),
        ]
    }

    // the ITPM sentinel marks a reservation; a channel that only carries OTPM keys is not one
    fn from_metadata(metadata: &Metadata) -> Option<Self> {
        metadata.get(ITPM_RESERVED_KEY)?;
        Some(Self {
            itpm_reserved: reserved_count(metadata.get(ITPM_RESERVED_KEY)),
            otpm_reserved: reserved_count(metadata.get(OTPM_RESERVED_KEY)),
            itpm_key: cache_key(metadata.get(ITPM_CACHE_KEY)),
            otpm_key: cache_key(metadata.get(OTPM_CACHE_KEY)),
        })
    }
}

// LiteLLM writes the sentinels into metadata (creating it) and into litellm_metadata when it is already a dict
pub fn stash_reservation(request: &mut Metadata, reservation: &IoReservation) {
    if let Some(Json::Object(metadata)) = request.get_mut("metadata") {
        insert_entries(metadata, reservation.entries());
    } else {
        let mut created = Metadata::new();
        insert_entries(&mut created, reservation.entries());
        request.insert("metadata".to_owned(), Json::Object(created));
    }
    if let Some(Json::Object(metadata)) = request.get_mut("litellm_metadata") {
        insert_entries(metadata, reservation.entries());
    }
}

// top-level metadata wins over litellm_metadata, then litellm_params.metadata, then standard_logging_object.metadata
#[must_use]
pub fn read_reservation(request: &Metadata) -> Option<IoReservation> {
    for channel in reservation_channels(request).into_iter().flatten() {
        if let Some(reservation) = IoReservation::from_metadata(channel) {
            return Some(reservation);
        }
    }
    None
}

// set_io_token_rate_limit_request_kwargs strips sentinels a caller put in metadata, so a forged reservation can't be replayed
pub fn clear_reservation(request: &mut Metadata) {
    for key in ["metadata", "litellm_metadata"] {
        if let Some(Json::Object(metadata)) = request.get_mut(key) {
            remove_sentinels(metadata);
        }
    }
    for key in ["litellm_params", "standard_logging_object"] {
        if let Some(Json::Object(record)) = request.get_mut(key)
            && let Some(Json::Object(metadata)) = record.get_mut("metadata")
        {
            remove_sentinels(metadata);
        }
    }
}

fn insert_entries(metadata: &mut Metadata, entries: [(&'static str, Json); 4]) {
    for (key, value) in entries {
        metadata.insert(key.to_owned(), value);
    }
}

fn reservation_channels(request: &Metadata) -> [Option<&Metadata>; 4] {
    [
        object(request.get("metadata")),
        object(request.get("litellm_metadata")),
        nested_metadata(request.get("litellm_params")),
        nested_metadata(request.get("standard_logging_object")),
    ]
}

const fn object(value: Option<&Json>) -> Option<&Metadata> {
    match value {
        Some(Json::Object(metadata)) => Some(metadata),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Null)
        | None => None,
    }
}

fn nested_metadata(value: Option<&Json>) -> Option<&Metadata> {
    match value {
        Some(Json::Object(record)) => object(record.get("metadata")),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Null)
        | None => None,
    }
}

fn remove_sentinels(metadata: &mut Metadata) {
    for key in RESERVATION_KEYS {
        metadata.shift_remove(key);
    }
}

fn reserved_count(value: Option<&Json>) -> i64 {
    match value {
        Some(Json::Number(number)) => number.as_i64().unwrap_or_default(),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) | Json::Null)
        | None => 0,
    }
}

fn cache_key(value: Option<&Json>) -> Option<String> {
    match value {
        Some(Json::Text(text)) => Some(text.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) | Json::Null)
        | None => None,
    }
}

fn text_or_null(value: Option<&String>) -> Json {
    let Some(text) = value else {
        return Json::Null;
    };
    Json::Text(text.clone())
}
