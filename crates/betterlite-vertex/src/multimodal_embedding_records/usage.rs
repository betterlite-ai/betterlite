use betterlite_models::decisions::Json;
use rust_decimal::Decimal;

use super::error::MultimodalEmbeddingError as Error;

const DURATION: &str = "video duration";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaPromptTokensDetails {
    pub character_count: Option<u64>,
    pub image_count: u64,
    pub video_length_seconds: Decimal,
}

pub fn calculate_usage(
    request: &Json,
    predictions: &[Json],
) -> Result<MediaPromptTokensDetails, Error> {
    let Json::Object(fields) = request else {
        return Err(Error::Request("instances", "a list"));
    };
    let Some(Json::List(instances)) = fields.get("instances") else {
        return Err(Error::Request("instances", "a list"));
    };
    let mut prompt = String::new();
    let mut has_text = false;
    let mut image_count = 0_u64;
    for instance in instances {
        let Json::Object(instance_fields) = instance else {
            return Err(Error::Request("instances", "a list of objects"));
        };
        match instance_fields.get("text") {
            Some(Json::Text(text)) if !text.is_empty() => {
                prompt.push_str(text);
                has_text = true;
            }
            Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
                return Err(Error::Request("text", "text"));
            }
            None | Some(Json::Null | Json::Text(_)) => {}
        }
        match instance_fields.get("image") {
            Some(Json::Object(image)) if !image.is_empty() => {
                image_count = image_count.saturating_add(1);
            }
            Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
                return Err(Error::Request("image", "an object"));
            }
            None | Some(Json::Null | Json::Object(_)) => {}
        }
    }
    Ok(MediaPromptTokensDetails {
        character_count: has_text.then(|| count_characters(&prompt)).transpose()?,
        image_count,
        video_length_seconds: video_length(predictions)?,
    })
}

fn count_characters(text: &str) -> Result<u64, Error> {
    // CPython's str.isspace also takes the four ASCII separators below the space character
    let count = text
        .chars()
        .filter(|character| {
            !matches!(*character, '\u{1c}'..='\u{1f}') && !character.is_whitespace()
        })
        .count();
    u64::try_from(count).map_err(|source| Error::IntegerRange("character count", source))
}

fn video_length(predictions: &[Json]) -> Result<Decimal, Error> {
    let mut total = Decimal::ZERO;
    for (index, prediction) in predictions.iter().enumerate() {
        let Json::Object(fields) = prediction else {
            return Err(Error::PredictionNotObject { index });
        };
        match fields.get("videoEmbeddings") {
            Some(Json::List(items)) => {
                let length = segment_length(items, index)?;
                total = total.checked_add(length).ok_or(Error::Overflow(DURATION))?;
            }
            Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
                return Err(Error::Field("videoEmbeddings", "list", index));
            }
            None | Some(Json::Null) => {}
        }
    }
    Ok(total)
}

fn segment_length(segments: &[Json], index: usize) -> Result<Decimal, Error> {
    let mut total = Decimal::ZERO;
    for segment in segments {
        let Json::Object(fields) = segment else {
            return Err(Error::PredictionNotObject { index });
        };
        let start = duration(fields.get("startOffsetSec"), "startOffsetSec", index)?;
        let end = duration(fields.get("endOffsetSec"), "endOffsetSec", index)?;
        let length = end.checked_sub(start).ok_or(Error::Overflow(DURATION))?;
        total = total.checked_add(length).ok_or(Error::Overflow(DURATION))?;
    }
    Ok(total)
}

fn duration(value: Option<&Json>, field: &'static str, index: usize) -> Result<Decimal, Error> {
    let Some(Json::Number(number)) = value else {
        return Err(Error::Field(field, "number", index));
    };
    let text = number.to_string();
    let parsed = if text.contains(['e', 'E']) {
        Decimal::from_scientific(&text)
    } else {
        text.parse()
    };
    parsed.map_err(|source| Error::Decimal(field, index, source))
}
