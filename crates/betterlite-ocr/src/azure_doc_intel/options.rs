use std::collections::BTreeSet;

use crate::json::{Json, Object};
use crate::ocr_error::OcrError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentIntelligenceParams {
    pub pages: Option<String>,
    pub features: Option<String>,
}

pub(super) fn map_params(params: &Object) -> Result<DocumentIntelligenceParams, OcrError> {
    Ok(DocumentIntelligenceParams {
        pages: normalize_pages(params.get("pages"))?,
        features: normalize_features(params.get("features"))?,
    })
}

fn normalize_pages(value: Option<&Json>) -> Result<Option<String>, OcrError> {
    let normalized = match value {
        None | Some(Json::Null) => return Ok(None),
        Some(Json::List(indexes)) if indexes.is_empty() => return Ok(None),
        Some(Json::List(indexes)) if indexes.iter().all(|item| matches!(item, Json::Number(_))) => {
            numeric_pages(indexes)?
        }
        Some(Json::List(tokens)) => {
            string_tokens(tokens, "expected only integers or only strings")?
        }
        Some(Json::Text(range)) => range
            .split(',')
            .map(str::trim)
            .collect::<Vec<_>>()
            .join(","),
        Some(Json::Bool(_) | Json::Number(_) | Json::Object(_)) => {
            return Err(OcrError::Pages(
                "expected an array of integers or strings, or a native page range".to_owned(),
            ));
        }
    };
    if !normalized.split(',').all(valid_page_token) {
        return Err(OcrError::Pages("invalid native page range".to_owned()));
    }
    Ok(Some(normalized))
}

fn numeric_pages(indexes: &[Json]) -> Result<String, OcrError> {
    let mut pages = BTreeSet::new();
    for index in indexes {
        let Json::Number(number) = index else {
            return Err(OcrError::Pages("page index is out of range".to_owned()));
        };
        let page = number
            .as_i64()
            .ok_or_else(|| OcrError::Pages("page index is out of range".to_owned()))?;
        if page < 0 {
            return Err(OcrError::Pages("negative page index".to_owned()));
        }
        let next = page
            .checked_add(1)
            .ok_or_else(|| OcrError::Pages("page index is out of range".to_owned()))?;
        let _inserted = pages.insert(next);
    }
    Ok(pages
        .into_iter()
        .map(|page| page.to_string())
        .collect::<Vec<_>>()
        .join(","))
}

fn string_tokens(tokens: &[Json], message: &str) -> Result<String, OcrError> {
    let mut parts = Vec::with_capacity(tokens.len());
    for token in tokens {
        let Json::Text(text) = token else {
            return Err(OcrError::Pages(message.to_owned()));
        };
        parts.push(text.trim());
    }
    Ok(parts.join(","))
}

fn valid_page_token(token: &str) -> bool {
    let mut parts = token.split('-');
    let start = parts.next().unwrap_or_default();
    if start.is_empty() || !start.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    parts.next().is_none_or(|end| {
        !end.is_empty() && end.bytes().all(|byte| byte.is_ascii_digit()) && parts.next().is_none()
    })
}

fn normalize_features(value: Option<&Json>) -> Result<Option<String>, OcrError> {
    let Some(present) = value else {
        return Ok(None);
    };
    joined_features(&feature_tokens(present)?)
}

fn joined_features(tokens: &[&str]) -> Result<Option<String>, OcrError> {
    let normalized: Vec<&str> = tokens.iter().map(|token| token.trim()).collect();
    if normalized.is_empty() {
        return Ok(None);
    }
    if normalized.iter().any(|token| !valid_feature(token)) {
        return Err(OcrError::Features);
    }
    Ok(Some(normalized.join(",")))
}

fn feature_tokens(value: &Json) -> Result<Vec<&str>, OcrError> {
    match value {
        Json::Null => Ok(Vec::new()),
        Json::List(names) => names
            .iter()
            .map(|name| match name {
                Json::Text(text) => Ok(text.as_str()),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                    Err(OcrError::Features)
                }
            })
            .collect(),
        Json::Text(names) => Ok(names.split(',').collect()),
        Json::Bool(_) | Json::Number(_) | Json::Object(_) => Err(OcrError::Features),
    }
}

fn valid_feature(token: &str) -> bool {
    let Some((first, rest)) = token.as_bytes().split_first() else {
        return false;
    };
    first.is_ascii_alphabetic() && rest.iter().all(u8::is_ascii_alphanumeric)
}
