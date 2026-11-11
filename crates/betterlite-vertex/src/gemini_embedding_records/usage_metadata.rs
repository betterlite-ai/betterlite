use betterlite_models::decisions::Json;

const INTEGER_FIELDS: [&str; 7] = [
    "cachedContentTokenCount",
    "candidatesTokenCount",
    "promptTokenCount",
    "responseTokenCount",
    "thoughtsTokenCount",
    "toolUsePromptTokenCount",
    "totalTokenCount",
];

const DETAIL_FIELDS: [&str; 5] = [
    "cacheTokensDetails",
    "candidatesTokensDetails",
    "promptTokensDetails",
    "responseTokensDetails",
    "toolUsePromptTokensDetails",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modality {
    Text,
    Audio,
    Image,
    Video,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModalityCount {
    pub modality: Modality,
    pub token_count: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageMetadata {
    pub prompt_token_count: Option<i64>,
    pub total_token_count: Option<i64>,
    pub prompt_tokens_details: Option<Vec<ModalityCount>>,
}

#[must_use]
pub fn parse_usage_metadata(raw: Option<&Json>) -> Option<UsageMetadata> {
    let Some(Json::Object(fields)) = raw else {
        return None;
    };
    let integers_valid = INTEGER_FIELDS
        .iter()
        .all(|field| integer(fields.get(*field)).is_some());
    let details_valid = DETAIL_FIELDS
        .iter()
        .all(|field| details(fields.get(*field)).is_some());
    if !integers_valid || !details_valid {
        return None;
    }
    Some(UsageMetadata {
        prompt_token_count: integer(fields.get("promptTokenCount"))??,
        total_token_count: integer(fields.get("totalTokenCount"))??,
        prompt_tokens_details: details(fields.get("promptTokensDetails"))??,
    })
}

fn integer(value: Option<&Json>) -> Option<Option<i64>> {
    match value {
        None | Some(Json::Null) => Some(None),
        Some(Json::Number(number)) => Some(Some(number.as_i64()?)),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => None,
    }
}

fn details(value: Option<&Json>) -> Option<Option<Vec<ModalityCount>>> {
    match value {
        None | Some(Json::Null) => Some(None),
        Some(Json::List(items)) => {
            let mut counts = Vec::with_capacity(items.len());
            for item in items {
                counts.push(modality_count(item)?);
            }
            Some(Some(counts))
        }
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => None,
    }
}

fn modality_count(value: &Json) -> Option<ModalityCount> {
    let Json::Object(fields) = value else {
        return None;
    };
    let Some(Json::Text(modality)) = fields.get("modality") else {
        return None;
    };
    let Some(Json::Number(count)) = fields.get("tokenCount") else {
        return None;
    };
    Some(ModalityCount {
        modality: modality_of(modality)?,
        token_count: count.as_i64()?,
    })
}

fn modality_of(value: &str) -> Option<Modality> {
    match value {
        "TEXT" => Some(Modality::Text),
        "AUDIO" => Some(Modality::Audio),
        "IMAGE" => Some(Modality::Image),
        "VIDEO" => Some(Modality::Video),
        _ => None,
    }
}
