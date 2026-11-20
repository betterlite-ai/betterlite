use std::fmt::{Debug, Formatter, Result as FmtResult};

use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use serde::Serialize;

pub mod agent;
pub mod aoss;
pub mod error;
pub mod query;
pub mod s3;

use error::CallError;

// RFC 3986 keeps only ALPHA, DIGIT and -_.~ as they are, in a path segment and in a form value
pub(super) const UNRESERVED: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'!')
    .add(b'"')
    .add(b'#')
    .add(b'$')
    .add(b'%')
    .add(b'&')
    .add(b'\'')
    .add(b'(')
    .add(b')')
    .add(b'*')
    .add(b'+')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

#[derive(Clone, PartialEq, Eq)]
pub struct CallRequest {
    pub operation: &'static str,
    pub service: &'static str,
    pub signing_region: String,
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

// the body can be a caller's file, so Debug names the headers and sizes the body
impl Debug for CallRequest {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        let names: Vec<&str> = self.headers.iter().map(|(name, _)| name.as_str()).collect();
        formatter
            .debug_struct("CallRequest")
            .field("operation", &self.operation)
            .field("service", &self.service)
            .field("signing_region", &self.signing_region)
            .field("method", &self.method)
            .field("url", &self.url)
            .field("header_names", &names)
            .field("body_length", &self.body.len())
            .finish()
    }
}

pub(super) fn encode_segment(value: &str) -> String {
    utf8_percent_encode(value, UNRESERVED).to_string()
}

// an AWS document field is a compact JSON string inside the request body
pub(super) fn document<Policy: Serialize>(
    policy: &Policy,
    operation: &'static str,
) -> Result<String, CallError> {
    serde_json::to_string(policy).map_err(|source| CallError::Serialize { operation, source })
}
