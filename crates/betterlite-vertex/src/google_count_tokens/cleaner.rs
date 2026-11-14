use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize};

use crate::google_count_tokens::error::GoogleCountError;

const FUNCTION_RESPONSE: &str = "functionResponse";
const PARTS: &str = "parts";

// the four names google-genai's FunctionResponseScheduling resolves case-insensitively
const SCHEDULING: [&str; 4] = ["SCHEDULING_UNSPECIFIED", "SILENT", "WHEN_IDLE", "INTERRUPT"];

// google-genai's FunctionResponse record: camelCase aliases in, snake_case fields out, an id
// cleared, None dropped, and a field the record does not declare is an error.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FunctionResponseRecord {
    #[serde(
        default,
        alias = "willContinue",
        skip_serializing_if = "Option::is_none"
    )]
    will_continue: Option<bool>,
    #[serde(
        default,
        deserialize_with = "canonical_scheduling",
        skip_serializing_if = "Option::is_none"
    )]
    scheduling: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parts: Option<Vec<Json>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    response: Option<IndexMap<String, Json>>,
}

// google-genai resolves a known scheduling name in any case and keeps an unknown one verbatim
fn canonical_scheduling<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    Ok(value.map(|text| {
        let upper = text.to_uppercase();
        if SCHEDULING.contains(&upper.as_str()) {
            upper
        } else {
            text
        }
    }))
}

// LiteLLM deep-copies the contents, projects every functionResponse through google-genai's
// FunctionResponse record with its id cleared, and drops the fields that come back None.
pub fn clean_contents(contents: &Json) -> Result<Json, GoogleCountError> {
    match contents {
        Json::Null => Ok(Json::Null),
        Json::List(records) => {
            let mut cleaned = Vec::with_capacity(records.len());
            for record in records {
                cleaned.push(clean_record(record)?);
            }
            Ok(Json::List(cleaned))
        }
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            Err(GoogleCountError::Contents {
                problem: "are not a list of content records",
            })
        }
    }
}

fn clean_record(record: &Json) -> Result<Json, GoogleCountError> {
    let Json::Object(fields) = record else {
        return Err(GoogleCountError::Contents {
            problem: "hold a content record that is not an object",
        });
    };
    let Some(parts) = fields.get(PARTS) else {
        return Err(GoogleCountError::Contents {
            problem: "hold a content record with no parts list",
        });
    };
    let Json::List(part_records) = parts else {
        return Err(GoogleCountError::Contents {
            problem: "hold parts that are not a list",
        });
    };
    let mut cleaned = Vec::with_capacity(part_records.len());
    for part in part_records {
        cleaned.push(clean_part(part)?);
    }
    let mut copied = fields.clone();
    copied.insert(PARTS.to_owned(), Json::List(cleaned));
    Ok(Json::Object(copied))
}

fn clean_part(part: &Json) -> Result<Json, GoogleCountError> {
    let Json::Object(fields) = part else {
        return Err(GoogleCountError::Contents {
            problem: "hold a part that is not an object",
        });
    };
    let Some(function_response) = fields.get(FUNCTION_RESPONSE) else {
        return Ok(Json::Object(fields.clone()));
    };
    let projected = project_function_response(function_response)?;
    let mut copied = fields.clone();
    copied.insert(FUNCTION_RESPONSE.to_owned(), projected);
    Ok(Json::Object(copied))
}

fn project_function_response(value: &Json) -> Result<Json, GoogleCountError> {
    let encoded = serde_json::to_vec(value).map_err(|source| GoogleCountError::Json {
        path: "the count request body".to_owned(),
        source,
    })?;
    let mut deserializer = serde_json::Deserializer::from_slice(&encoded);
    let mut record: FunctionResponseRecord = serde_path_to_error::deserialize(&mut deserializer)
        .map_err(|error| GoogleCountError::FunctionResponse {
            path: if error.path().is_empty() {
                "the request contents".to_owned()
            } else {
                error.path().to_string()
            },
            source: error.into_inner(),
        })?;
    record.id = None;
    let projected = serde_json::to_vec(&record).map_err(|source| GoogleCountError::Json {
        path: "the functionResponse record".to_owned(),
        source,
    })?;
    serde_json::from_slice(&projected).map_err(|source| GoogleCountError::Json {
        path: "the functionResponse record".to_owned(),
        source,
    })
}
