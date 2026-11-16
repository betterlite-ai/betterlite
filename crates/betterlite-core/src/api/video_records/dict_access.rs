use betterlite_models::decisions::Json;
use betterlite_openai::video_edits::response::{CharacterKind, VideoKind};
use indexmap::IndexMap;
use serde_json::Number;

use super::{CharacterObject, VideoObject, VideoRecordError};

const HIDDEN_PARAMS: &str = "_hidden_params";

// Python's __contains__, get, __getitem__ and json on the two pydantic models, all reading the declared fields plus the private _hidden_params that only the attribute methods see.
pub trait DictAccess {
    const FIELDS: &'static [&'static str];
    const OBJECT: &'static str;

    #[must_use]
    fn field(&self, name: &str) -> Option<Json>;

    #[must_use]
    fn contains(&self, key: &str) -> bool {
        self.field(key).is_some()
    }

    #[must_use]
    fn get(&self, key: &str) -> Option<Json> {
        self.field(key)
    }

    fn getitem(&self, key: &str) -> Result<Json, VideoRecordError> {
        let Some(value) = self.field(key) else {
            return Err(VideoRecordError::UnknownField {
                object: Self::OBJECT,
                field: key.to_owned(),
            });
        };
        Ok(value)
    }

    #[must_use]
    fn json(&self) -> IndexMap<String, Json> {
        let mut fields = IndexMap::new();
        for name in Self::FIELDS {
            if let Some(value) = self.field(name) {
                fields.insert((*name).to_owned(), value);
            }
        }
        fields
    }
}

impl DictAccess for VideoObject {
    const FIELDS: &'static [&'static str] = &[
        "id",
        "object",
        "status",
        "created_at",
        "completed_at",
        "expires_at",
        "error",
        "progress",
        "remixed_from_video_id",
        "seconds",
        "size",
        "model",
        "usage",
    ];
    const OBJECT: &'static str = "VideoObject";

    fn field(&self, name: &str) -> Option<Json> {
        match name {
            "id" => Some(Json::Text(self.id.clone())),
            "object" => Some(Json::Text(match self.object {
                VideoKind::Video => "video".to_owned(),
            })),
            "status" => Some(Json::Text(self.status.clone())),
            "created_at" => Some(optional_integer(self.created_at)),
            "completed_at" => Some(optional_integer(self.completed_at)),
            "expires_at" => Some(optional_integer(self.expires_at)),
            "error" => Some(optional_object(self.error.as_ref())),
            "progress" => Some(optional_integer(self.progress)),
            "remixed_from_video_id" => Some(optional_text(self.remixed_from_video_id.as_deref())),
            "seconds" => Some(optional_text(self.seconds.as_deref())),
            "size" => Some(optional_text(self.size.as_deref())),
            "model" => Some(optional_text(self.model.as_deref())),
            "usage" => Some(optional_object(self.usage.as_ref())),
            HIDDEN_PARAMS => Some(Json::Object(self.hidden_params.clone())),
            _ => None,
        }
    }
}

impl DictAccess for CharacterObject {
    const FIELDS: &'static [&'static str] = &["id", "object", "created_at", "name"];
    const OBJECT: &'static str = "CharacterObject";

    fn field(&self, name: &str) -> Option<Json> {
        match name {
            "id" => Some(Json::Text(self.id.clone())),
            "object" => Some(Json::Text(match self.object {
                CharacterKind::Character => "character".to_owned(),
            })),
            "created_at" => Some(Json::Number(Number::from(self.created_at))),
            "name" => Some(Json::Text(self.name.clone())),
            HIDDEN_PARAMS => Some(Json::Object(self.hidden_params.clone())),
            _ => None,
        }
    }
}

fn optional_integer(value: Option<i64>) -> Json {
    match value {
        Some(number) => Json::Number(Number::from(number)),
        None => Json::Null,
    }
}

fn optional_text(value: Option<&str>) -> Json {
    match value {
        Some(text) => Json::Text(text.to_owned()),
        None => Json::Null,
    }
}

fn optional_object(value: Option<&IndexMap<String, Json>>) -> Json {
    match value {
        Some(fields) => Json::Object(fields.clone()),
        None => Json::Null,
    }
}
