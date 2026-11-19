use serde::de::DeserializeOwned;
use serde_json::error::Category;
use serde_path_to_error::Segment;

use super::error::{
    FIELD_REQUIRED, JSON_DECODE_ERROR, MODEL_ATTRIBUTES_MESSAGE, RoiRoutesError, issue,
};

pub(super) fn decode_body<T>(body: &[u8]) -> Result<T, RoiRoutesError>
where
    T: DeserializeOwned,
{
    if body.is_empty() {
        return Err(RoiRoutesError::Request(vec![issue(
            "missing",
            vec!["body"],
            FIELD_REQUIRED,
        )]));
    }
    let mut deserializer = serde_json::Deserializer::from_slice(body);
    let value = serde_path_to_error::deserialize(&mut deserializer)
        .map_err(|source| body_error(&source))?;
    deserializer.end().map_err(|_error| {
        RoiRoutesError::Request(vec![issue("json_invalid", vec!["body"], JSON_DECODE_ERROR)])
    })?;
    Ok(value)
}

fn body_error(source: &serde_path_to_error::Error<serde_json::Error>) -> RoiRoutesError {
    if matches!(source.inner().classify(), Category::Syntax | Category::Eof) {
        return RoiRoutesError::Request(vec![issue(
            "json_invalid",
            vec!["body"],
            JSON_DECODE_ERROR,
        )]);
    }
    let message = source.inner().to_string();
    if let Some(rest) = message.strip_prefix("unknown field `")
        && let Some((field, _rest)) = rest.split_once('`')
    {
        return RoiRoutesError::UnknownField {
            field: field.to_owned(),
        };
    }
    if let Some(field) = missing_field(&message) {
        return RoiRoutesError::Request(vec![issue(
            "missing",
            vec!["body", field],
            FIELD_REQUIRED,
        )]);
    }
    let mut loc = vec!["body"];
    for segment in source.path() {
        if let Segment::Map { key } = segment
            && let Some(name) = known_field(key)
        {
            loc.push(name);
        }
    }
    if loc.len() == 1 {
        return RoiRoutesError::Request(vec![issue(
            "model_attributes_type",
            vec!["body"],
            MODEL_ATTRIBUTES_MESSAGE,
        )]);
    }
    RoiRoutesError::Request(vec![issue("value_error", loc, &message)])
}

fn missing_field(message: &str) -> Option<&'static str> {
    let rest = message.strip_prefix("missing field `")?;
    let (field, _rest) = rest.split_once('`')?;
    known_field(field)
}

fn known_field(key: &str) -> Option<&'static str> {
    match key {
        "github_login" => Some("github_login"),
        "email" => Some("email"),
        _ => None,
    }
}
