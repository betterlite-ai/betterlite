use axum::Json;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use std::collections::BTreeSet;

use crate::list_api::plan::types::FilterOp;
use crate::management_envelopes::{ProblemDetail, ProblemDocument};
mod tests;

pub const PROBLEM_CONTENT_TYPE: &str = "application/problem+json";
const PROBLEM_TYPE_BASE: &str = "urn:litellm:error:";

#[derive(Debug, thiserror::Error)]
#[error("{}", .0.detail)]
pub struct ManagementProblem(pub ProblemDetail);

impl IntoResponse for ManagementProblem {
    fn into_response(self) -> Response {
        problem_response(&self.0)
    }
}

#[must_use]
pub fn problem_response(problem: &ProblemDetail) -> Response {
    let status = StatusCode::from_u16(problem.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut response = (status, Json(ProblemDocument::from(problem))).into_response();
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(PROBLEM_CONTENT_TYPE));
    response
}

fn problem(
    slug: &str,
    title: &str,
    status: u16,
    detail: String,
    allowed: Option<Vec<String>>,
) -> ProblemDetail {
    ProblemDetail {
        r#type: format!("{PROBLEM_TYPE_BASE}{slug}"),
        title: title.to_owned(),
        status,
        detail,
        allowed: allowed.map(|mut names| {
            names.sort();
            names
        }),
    }
}

#[must_use]
pub fn invalid_query_parameter(detail: String) -> ProblemDetail {
    problem(
        "invalid-query-parameter",
        "Invalid query parameter",
        400,
        detail,
        None,
    )
}

#[derive(Clone, Copy)]
pub enum SortProblem<'borrow> {
    Invalid { resource: &'borrow str },
    Duplicate,
}

#[must_use]
pub fn sort_problem(
    kind: SortProblem<'_>,
    fields: &BTreeSet<&str>,
    sortable: &BTreeSet<String>,
) -> ProblemDetail {
    let names = fields.iter().copied().collect::<Vec<_>>().join(", ");
    let (slug, title, detail) = match kind {
        SortProblem::Invalid { resource } => (
            "invalid-sort-field",
            "Invalid sort field",
            format!("Cannot sort {resource} by: {names}."),
        ),
        SortProblem::Duplicate => (
            "duplicate-sort-field",
            "Duplicate sort field",
            format!("Sort field(s) named more than once: {names}. Each may appear once."),
        ),
    };
    problem(
        slug,
        title,
        400,
        detail,
        Some(sortable.iter().cloned().collect()),
    )
}

#[must_use]
pub fn unsupported_filter_operator(
    field: &str,
    op: FilterOp,
    ops: &BTreeSet<FilterOp>,
) -> ProblemDetail {
    let allowed: Vec<String> = ops
        .iter()
        .map(|candidate| candidate.as_wire_name().to_owned())
        .collect();
    problem(
        "unsupported-filter-operator",
        "Unsupported filter operator",
        400,
        format!(
            "Operator '{}' is not supported on '{field}'.",
            op.as_wire_name()
        ),
        Some(allowed),
    )
}

#[must_use]
pub fn unknown_query_parameter(unknown: &BTreeSet<&str>, allowed: Vec<String>) -> ProblemDetail {
    let names: Vec<&str> = unknown.iter().copied().collect();
    problem(
        "unknown-query-parameter",
        "Unknown query parameter",
        400,
        format!("Unrecognized query parameter(s): {}.", names.join(", ")),
        Some(allowed),
    )
}

pub fn reject_unknown_query_params(
    received: &[String],
    declared: &BTreeSet<&str>,
) -> Result<(), ManagementProblem> {
    let unknown: BTreeSet<&str> = received
        .iter()
        .map(String::as_str)
        .filter(|name| !declared.contains(name))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    let allowed: Vec<String> = declared.iter().map(|name| (*name).to_owned()).collect();
    Err(ManagementProblem(unknown_query_parameter(
        &unknown, allowed,
    )))
}

#[must_use]
pub fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum LocPart {
    Field(String),
    Index(usize),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ValidationErrorDetail {
    pub error_type: String,
    pub loc: Vec<LocPart>,
    pub msg: String,
}

fn loc_part_text(part: &LocPart) -> String {
    match part {
        LocPart::Field(name) => name.clone(),
        LocPart::Index(index) => index.to_string(),
    }
}

fn suppress_parent_length(error: &ValidationErrorDetail, errors: &[ValidationErrorDetail]) -> bool {
    error.error_type == "too_short"
        && errors.iter().any(|other| {
            other.loc.len() > error.loc.len()
                && other.loc.get(..error.loc.len()) == Some(error.loc.as_slice())
        })
}

fn joined_error_detail(errors: &[&ValidationErrorDetail]) -> String {
    errors
        .iter()
        .map(|error| {
            let path: Vec<String> = error.loc.iter().skip(1).map(loc_part_text).collect();
            format!("{}: {}", path.join("."), error.msg)
        })
        .collect::<Vec<_>>()
        .join("; ")
}

#[must_use]
pub fn request_validation_problem(raw_errors: &[ValidationErrorDetail]) -> ProblemDetail {
    let errors: Vec<&ValidationErrorDetail> = raw_errors
        .iter()
        .filter(|error| !suppress_parent_length(error, raw_errors))
        .collect();
    let (slug, title, status, fallback) = if errors
        .iter()
        .any(|error| matches!(error.loc.first(), Some(LocPart::Field(field)) if field == "body"))
    {
        (
            "invalid-request-body",
            "Invalid request body",
            422,
            "The request body is invalid.",
        )
    } else {
        (
            "invalid-query-parameter",
            "Invalid query parameter",
            400,
            "The request query parameters are invalid.",
        )
    };
    let detail = joined_error_detail(&errors);
    problem(
        slug,
        title,
        status,
        if detail.is_empty() {
            fallback.to_owned()
        } else {
            detail
        },
        None,
    )
}
