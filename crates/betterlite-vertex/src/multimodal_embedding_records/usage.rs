use betterlite_models::decisions::Extra;
use betterlite_models::decisions::Json;
use rust_decimal::Decimal;

use super::error::MultimodalEmbeddingError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaPromptTokensDetails {
    pub character_count: Option<u64>,
    pub image_count: u64,
    pub video_length_seconds: Decimal,
}

pub fn calculate_usage(
    request: &Json,
    predictions: &[Json],
) -> Result<MediaPromptTokensDetails, MultimodalEmbeddingError> {
    let instances = instances(request)?;
    let mut prompt: Option<String> = None;
    let mut image_count = 0_u64;
    for instance in instances {
        let Json::Object(fields) = instance else {
            return Err(MultimodalEmbeddingError::InstanceNotObject);
        };
        append_text(fields, &mut prompt)?;
        if has_image(fields)? {
            image_count = image_count.saturating_add(1);
        }
    }
    let character_count = match prompt {
        Some(text) => Some(count_characters(&text)?),
        None => None,
    };
    let video_length_seconds = video_length(predictions)?;
    Ok(MediaPromptTokensDetails {
        character_count,
        image_count,
        video_length_seconds,
    })
}

fn instances(request: &Json) -> Result<&[Json], MultimodalEmbeddingError> {
    let Json::Object(fields) = request else {
        return Err(MultimodalEmbeddingError::InstancesNotList);
    };
    let Some(Json::List(instances)) = fields.get("instances") else {
        return Err(MultimodalEmbeddingError::InstancesNotList);
    };
    Ok(instances)
}

fn append_text(
    fields: &Extra,
    prompt: &mut Option<String>,
) -> Result<(), MultimodalEmbeddingError> {
    match fields.get("text") {
        None | Some(Json::Null) => Ok(()),
        Some(Json::Text(text)) if text.is_empty() => Ok(()),
        Some(Json::Text(text)) => {
            match prompt {
                Some(existing) => existing.push_str(text),
                None => *prompt = Some(text.clone()),
            }
            Ok(())
        }
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(MultimodalEmbeddingError::NotText { field: "text" })
        }
    }
}

fn has_image(fields: &Extra) -> Result<bool, MultimodalEmbeddingError> {
    match fields.get("image") {
        None | Some(Json::Null) => Ok(false),
        Some(Json::Object(image)) => Ok(!image.is_empty()),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(MultimodalEmbeddingError::NotObject { field: "image" })
        }
    }
}

fn count_characters(text: &str) -> Result<u64, MultimodalEmbeddingError> {
    // CPython's str.isspace also takes the four ASCII separators below the space character
    u64::try_from(
        text.chars()
            .filter(|character| {
                !matches!(*character, '\u{1c}'..='\u{1f}') && !character.is_whitespace()
            })
            .count(),
    )
    .map_err(|_source| MultimodalEmbeddingError::CountOverflow)
}

fn video_length(predictions: &[Json]) -> Result<Decimal, MultimodalEmbeddingError> {
    let mut total = Decimal::ZERO;
    for (index, prediction) in predictions.iter().enumerate() {
        let Json::Object(fields) = prediction else {
            return Err(MultimodalEmbeddingError::PredictionNotObject { index });
        };
        match fields.get("videoEmbeddings") {
            None | Some(Json::Null) => {}
            Some(Json::List(segments)) => {
                total = accumulated(total, segments, index)?;
            }
            Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
                return Err(MultimodalEmbeddingError::NotVector {
                    field: "videoEmbeddings",
                    index,
                });
            }
        }
    }
    Ok(total)
}

fn accumulated(
    mut total: Decimal,
    segments: &[Json],
    index: usize,
) -> Result<Decimal, MultimodalEmbeddingError> {
    for segment in segments {
        let Json::Object(fields) = segment else {
            return Err(MultimodalEmbeddingError::PredictionNotObject { index });
        };
        let start = duration(fields.get("startOffsetSec"), "startOffsetSec", index)?;
        let end = duration(fields.get("endOffsetSec"), "endOffsetSec", index)?;
        let length = end
            .checked_sub(start)
            .ok_or(MultimodalEmbeddingError::DurationOverflow)?;
        total = total
            .checked_add(length)
            .ok_or(MultimodalEmbeddingError::DurationOverflow)?;
    }
    Ok(total)
}

fn duration(
    value: Option<&Json>,
    field: &'static str,
    index: usize,
) -> Result<Decimal, MultimodalEmbeddingError> {
    let Some(Json::Number(number)) = value else {
        return Err(MultimodalEmbeddingError::NotNumber { field, index });
    };
    number
        .to_string()
        .parse::<Decimal>()
        .map_err(|_source| MultimodalEmbeddingError::NotNumber { field, index })
}
