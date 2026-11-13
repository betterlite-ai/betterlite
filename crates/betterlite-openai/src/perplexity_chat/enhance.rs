mod annotations;

use serde_json::{Map, Value};
use thiserror::Error;

use super::json_truthy;

#[derive(Debug, Error)]
pub(crate) enum PerplexityReplyError {
    #[error("the provider reply is not valid JSON")]
    RawJson(#[source] serde_json::Error),
    #[error("the reply is not a JSON object")]
    NotAnObject,
    #[error("the provider reply's {field} has an unexpected shape")]
    Shape { field: &'static str },
    #[error("the citation character count overflowed")]
    Overflow,
    #[error("the citation marker pattern is invalid")]
    MarkerPattern(#[source] fancy_regex::Error),
    #[error("the citation marker could not be read")]
    Marker(#[source] fancy_regex::Error),
}

// Both halves are computed before anything is written, so any failure leaves the reply as it was
pub(crate) fn apply(reply: &mut Value, raw_body: &[u8]) -> Result<(), PerplexityReplyError> {
    let raw: Value = serde_json::from_slice(raw_body).map_err(PerplexityReplyError::RawJson)?;
    let Value::Object(raw_object) = &raw else {
        return Err(PerplexityReplyError::NotAnObject);
    };
    let reply_object = reply
        .as_object_mut()
        .ok_or(PerplexityReplyError::NotAnObject)?;
    let usage = usage_plan(reply_object, raw_object)?;
    let annotations = annotations::plan(reply_object, raw_object)?;
    usage.apply(reply_object);
    annotations.apply(reply_object);
    Ok(())
}

struct UsagePlan {
    create_details: bool,
    citation_tokens: Option<i64>,
    search_requests: Option<i64>,
}

impl UsagePlan {
    fn apply(self, reply: &mut Map<String, Value>) {
        if !matches!(reply.get("usage"), Some(Value::Object(_))) {
            reply.insert(String::from("usage"), zero_usage());
        }
        let Some(Value::Object(usage)) = reply.get_mut("usage") else {
            return;
        };
        if self.create_details
            && !matches!(usage.get("prompt_tokens_details"), Some(Value::Object(_)))
        {
            usage.insert(
                String::from("prompt_tokens_details"),
                Value::Object(Map::new()),
            );
        }
        if let Some(tokens) = self.citation_tokens {
            usage.insert(String::from("citation_tokens"), Value::from(tokens));
        }
        if let Some(count) = self.search_requests
            && let Some(Value::Object(details)) = usage.get_mut("prompt_tokens_details")
        {
            details.insert(String::from("web_search_requests"), Value::from(count));
        }
    }
}

fn zero_usage() -> Value {
    Value::Object(Map::from_iter([
        (String::from("prompt_tokens"), Value::from(0)),
        (String::from("completion_tokens"), Value::from(0)),
        (String::from("total_tokens"), Value::from(0)),
    ]))
}

fn usage_plan(
    reply: &Map<String, Value>,
    raw: &Map<String, Value>,
) -> Result<UsagePlan, PerplexityReplyError> {
    let raw_usage = match raw.get("usage") {
        None => None,
        Some(Value::Object(usage)) => Some(usage),
        Some(Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_))
        | Some(Value::Array(_)) => {
            // the pinned code calls .get on whatever the usage key holds and fails on a non-map
            return Err(PerplexityReplyError::Shape { field: "usage" });
        }
    };
    if let Some(usage) = reply.get("usage")
        && !usage.is_object()
        && !usage.is_null()
    {
        return Err(PerplexityReplyError::Shape { field: "usage" });
    }
    let citation_tokens = citation_tokens(raw)?;
    let count = search_count(raw_usage, raw)?;
    Ok(UsagePlan {
        create_details: citation_tokens > 0 || count.is_some_and(|count| count > 0),
        citation_tokens: (citation_tokens > 0).then_some(citation_tokens),
        search_requests: count.filter(|count| *count > 0),
    })
}

fn citation_tokens(raw: &Map<String, Value>) -> Result<i64, PerplexityReplyError> {
    let entries = match raw.get("citations") {
        None | Some(Value::Null) => &[][..],
        Some(Value::Array(entries)) => entries.as_slice(),
        Some(Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Object(_)) => {
            return Err(PerplexityReplyError::Shape { field: "citations" });
        }
    };
    let mut total: i64 = 0;
    for entry in entries.iter().filter(|entry| json_truthy(entry)) {
        let characters = match entry {
            Value::String(text) => text.chars().count(),
            Value::Null
            | Value::Bool(_)
            | Value::Number(_)
            | Value::Array(_)
            | Value::Object(_) => entry.to_string().chars().count(),
        };
        let characters =
            i64::try_from(characters).map_err(|_error| PerplexityReplyError::Overflow)?;
        total = total
            .checked_add(characters)
            .ok_or(PerplexityReplyError::Overflow)?;
    }
    if total <= 0 {
        return Ok(0);
    }
    let quarter = total.checked_div(4).ok_or(PerplexityReplyError::Overflow)?;
    Ok(quarter.max(1))
}

fn search_count(
    raw_usage: Option<&Map<String, Value>>,
    raw: &Map<String, Value>,
) -> Result<Option<i64>, PerplexityReplyError> {
    let candidates = [
        raw_usage.and_then(|usage| usage.get("num_search_queries")),
        raw.get("num_search_queries"),
        raw_usage.and_then(|usage| usage.get("search_queries")),
        raw.get("search_queries"),
    ];
    for candidate in candidates {
        match candidate {
            None | Some(Value::Null) => continue,
            Some(Value::Number(number)) => {
                return number
                    .as_i64()
                    .map(Some)
                    .ok_or(PerplexityReplyError::Shape {
                        field: "num_search_queries",
                    });
            }
            Some(Value::Bool(_) | Value::String(_) | Value::Array(_) | Value::Object(_)) => {
                return Err(PerplexityReplyError::Shape {
                    field: "num_search_queries",
                });
            }
        }
    }
    Ok(None)
}
