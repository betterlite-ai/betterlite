use betterlite_config::sections::settings::JsonValue;
use serde::Serialize;

use super::{BODY, MESSAGES};

const REQUIRED: &str = "Field required";
const DICTIONARY: &str = "Input should be a valid dictionary or object to extract fields from";
const LIST: &str = "Input should be a valid list";
const STRING: &str = "Input should be a valid string";
const ROLE_LITERAL: &str = "Input should be 'user' or 'assistant'";

#[derive(Debug, Serialize)]
pub struct ValidationIssue {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub loc: Vec<LocPart>,
    pub msg: &'static str,
    pub input: JsonValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ctx: Option<IssueContext>,
}

#[derive(Clone, Copy)]
pub enum IssueKind {
    Missing,
    NotObject,
    NotList,
    NotText,
    NotOneOf { expected: &'static str },
}

impl ValidationIssue {
    // pydantic's error table: the type, message and context each kind reports
    #[must_use]
    pub const fn of(kind: IssueKind, loc: Vec<LocPart>, input: JsonValue) -> Self {
        let (kind, msg, ctx) = match kind {
            IssueKind::Missing => ("missing", REQUIRED, None),
            IssueKind::NotObject => ("model_attributes_type", DICTIONARY, None),
            IssueKind::NotList => ("list_type", LIST, None),
            IssueKind::NotText => ("string_type", STRING, None),
            IssueKind::NotOneOf { expected } => (
                "literal_error",
                ROLE_LITERAL,
                Some(IssueContext::Expected { expected }),
            ),
        };
        Self {
            kind,
            loc,
            msg,
            input,
            ctx,
        }
    }

    // pydantic locates a field of a chat message under the body's messages list
    #[must_use]
    pub fn at_field(kind: IssueKind, index: usize, name: &'static str, input: JsonValue) -> Self {
        let loc = vec![
            LocPart::Field(BODY),
            LocPart::Field(MESSAGES),
            LocPart::Index(index),
            LocPart::Field(name),
        ];
        Self::of(kind, loc, input)
    }
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum IssueContext {
    Expected { expected: &'static str },
    Error { error: String },
}

#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum LocPart {
    Field(&'static str),
    Index(usize),
}
