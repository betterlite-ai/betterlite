use std::collections::BTreeMap;

use serde::Serialize;

const SESSION_ID_PARAM: &str = "session_id";
const PAGE_PARAM: &str = "page";
const PAGE_SIZE_PARAM: &str = "page_size";
const DEFAULT_PAGE: i64 = 1;
const DEFAULT_PAGE_SIZE: i64 = 50;
const MAX_PAGE_SIZE: i64 = 100;

const REQUIRED_MESSAGE: &str = "Field required";
const INTEGER_MESSAGE: &str =
    "Input should be a valid integer, unable to parse string as an integer";

// ui_view_session_spend_logs reads one exact session id, without a time window
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionExpansionQuery {
    pub session_id: String,
    pub page: i64,
    pub page_size: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionQueryIssue {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub loc: [&'static str; 2],
    pub msg: String,
}

// the framework's 422 body, rebuilt by the repository convention without pydantic's input and ctx
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionQueryIssues {
    pub detail: Vec<SessionQueryIssue>,
}

#[derive(Debug, Clone, Copy)]
struct BoundedInteger {
    name: &'static str,
    fallback: i64,
    minimum: i64,
    maximum: Option<i64>,
}

const PAGE_BOUND: BoundedInteger = BoundedInteger {
    name: PAGE_PARAM,
    fallback: DEFAULT_PAGE,
    minimum: DEFAULT_PAGE,
    maximum: None,
};

const PAGE_SIZE_BOUND: BoundedInteger = BoundedInteger {
    name: PAGE_SIZE_PARAM,
    fallback: DEFAULT_PAGE_SIZE,
    minimum: DEFAULT_PAGE,
    maximum: Some(MAX_PAGE_SIZE),
};

#[derive(Debug, Clone, Copy)]
enum ParsedInteger {
    Value(i64),
    AboveSignedRange,
    Invalid,
}

// FastAPI validates the endpoint's own query parameters in their declared order
#[must_use]
pub fn decode_session_query(
    raw_query: Option<&str>,
) -> Result<SessionExpansionQuery, SessionQueryIssues> {
    let values: BTreeMap<String, String> =
        form_urlencoded::parse(raw_query.unwrap_or_default().as_bytes())
            .into_owned()
            .collect();
    let mut issues = Vec::new();
    let session_id = values.get(SESSION_ID_PARAM).cloned();
    if session_id.is_none() {
        issues.push(SessionQueryIssue {
            kind: "missing",
            loc: ["query", SESSION_ID_PARAM],
            msg: String::from(REQUIRED_MESSAGE),
        });
    }
    let page = decode_bounded(&values, PAGE_BOUND, &mut issues);
    let page_size = decode_bounded(&values, PAGE_SIZE_BOUND, &mut issues);
    match (session_id, issues.is_empty()) {
        (Some(session_id), true) => Ok(SessionExpansionQuery {
            session_id,
            page,
            page_size,
        }),
        _ => Err(SessionQueryIssues { detail: issues }),
    }
}

fn decode_bounded(
    values: &BTreeMap<String, String>,
    bound: BoundedInteger,
    issues: &mut Vec<SessionQueryIssue>,
) -> i64 {
    let Some(text) = values.get(bound.name) else {
        return bound.fallback;
    };
    let value = match parse_integer(text) {
        ParsedInteger::Value(value) => value,
        ParsedInteger::AboveSignedRange => {
            let issue = bound.maximum.map_or_else(
                || parse_issue(bound.name),
                |maximum| above_issue(bound.name, maximum),
            );
            issues.push(issue);
            return bound.fallback;
        }
        ParsedInteger::Invalid => {
            issues.push(parse_issue(bound.name));
            return bound.fallback;
        }
    };
    if value < bound.minimum {
        issues.push(SessionQueryIssue {
            kind: "greater_than_equal",
            loc: ["query", bound.name],
            msg: format!("Input should be greater than or equal to {}", bound.minimum),
        });
        return bound.fallback;
    }
    if let Some(maximum) = bound.maximum
        && value > maximum
    {
        issues.push(above_issue(bound.name, maximum));
        return bound.fallback;
    }
    value
}

fn parse_issue(name: &'static str) -> SessionQueryIssue {
    SessionQueryIssue {
        kind: "int_parsing",
        loc: ["query", name],
        msg: String::from(INTEGER_MESSAGE),
    }
}

fn above_issue(name: &'static str, maximum: i64) -> SessionQueryIssue {
    SessionQueryIssue {
        kind: "less_than_equal",
        loc: ["query", name],
        msg: format!("Input should be less than or equal to {maximum}"),
    }
}

// pydantic reads an integer from text: surrounding space, a sign, digit groups joined by single underscores and a zeros-only fraction, while a magnitude past i64 keeps the sign it was written with and a positive one answers the declared bound
fn parse_integer(text: &str) -> ParsedInteger {
    let trimmed = text.trim();
    let (negative, unsigned) = match trimmed.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
    };
    let (whole, fraction) = match unsigned.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (unsigned, None),
    };
    if let Some(digits) = fraction
        && (digits.is_empty() || !digits.bytes().all(|byte| byte == b'0'))
    {
        return ParsedInteger::Invalid;
    }
    let mut after_digit = false;
    for byte in whole.bytes() {
        if byte == b'_' && after_digit {
            after_digit = false;
        } else if byte.is_ascii_digit() {
            after_digit = true;
        } else {
            return ParsedInteger::Invalid;
        }
    }
    if !after_digit {
        return ParsedInteger::Invalid;
    }
    match whole.replace('_', "").parse::<i64>() {
        Ok(magnitude) if negative => ParsedInteger::Value(0_i64.saturating_sub(magnitude)),
        Ok(magnitude) => ParsedInteger::Value(magnitude),
        Err(_error) if negative => ParsedInteger::Value(i64::MIN),
        Err(_error) => ParsedInteger::AboveSignedRange,
    }
}
