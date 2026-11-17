use serde::Serialize;

use super::{BODY, MESSAGES};

const REQUIRED: &str = "Field required";
const DECODE: &str = "JSON decode error";
const DICTIONARY: &str = "Input should be a valid dictionary or object to extract fields from";
const LIST: &str = "Input should be a valid list";
const STRING: &str = "Input should be a valid string";
const ROLE_LITERAL: &str = "Input should be 'user' or 'assistant'";

// LiteLLM's RequestValidationError handler sends only type, loc and msg, without input or ctx
#[derive(Debug, Serialize)]
pub struct ValidationIssue {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub loc: Vec<LocPart>,
    pub msg: &'static str,
}

#[derive(Clone, Copy)]
pub enum IssueKind {
    Missing,
    NotObject,
    NotList,
    NotText,
    NotOneOf,
    JsonInvalid,
}

impl ValidationIssue {
    // pydantic's error table: the type and message each kind reports
    #[must_use]
    pub const fn of(kind: IssueKind, loc: Vec<LocPart>) -> Self {
        let (issue_type, message) = match kind {
            IssueKind::Missing => ("missing", REQUIRED),
            IssueKind::NotObject => ("model_attributes_type", DICTIONARY),
            IssueKind::NotList => ("list_type", LIST),
            IssueKind::NotText => ("string_type", STRING),
            IssueKind::NotOneOf => ("literal_error", ROLE_LITERAL),
            IssueKind::JsonInvalid => ("json_invalid", DECODE),
        };
        Self {
            kind: issue_type,
            loc,
            msg: message,
        }
    }

    // pydantic locates a field of a chat message under the body's messages list
    #[must_use]
    pub fn at_field(kind: IssueKind, index: usize, name: &'static str) -> Self {
        let loc = vec![
            LocPart::Field(BODY),
            LocPart::Field(MESSAGES),
            LocPart::Index(index),
            LocPart::Field(name),
        ];
        Self::of(kind, loc)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum LocPart {
    Field(&'static str),
    Index(usize),
}
