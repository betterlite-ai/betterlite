use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use super::model::{Step, StepCompare};
use super::run::StepCapture;
use crate::case::Compare;
use crate::compare::{self, Difference as ResponseDifference, remove_pointer};
use crate::http::Response;
use crate::mock::ProviderRequest;

#[derive(Debug, Serialize)]
pub(crate) struct Difference {
    pub(crate) message: String,
    pub(crate) pointer: String,
    pub(crate) reference: Value,
    pub(crate) rust: Value,
}

#[derive(Serialize)]
pub(crate) struct Side<'input> {
    status: u16,
    headers: &'input BTreeMap<String, String>,
    body: &'input Value,
    provider: &'input [ProviderRequest],
}

pub(crate) fn side(capture: &StepCapture) -> Side<'_> {
    Side {
        status: capture.response.status,
        headers: &capture.response.headers,
        body: &capture.response.body,
        provider: &capture.provider,
    }
}

pub(crate) fn compare_transcripts(
    first: &[StepCapture],
    second: &[StepCapture],
    steps: &[Step],
) -> Result<(), Difference> {
    if first.len() != steps.len() || second.len() != steps.len() {
        return Err(Difference {
            message: "a scenario step was not captured".to_owned(),
            pointer: "/steps".to_owned(),
            reference: Value::from(first.len()),
            rust: Value::from(second.len()),
        });
    }
    for (index, ((first_step, second_step), step)) in
        first.iter().zip(second).zip(steps).enumerate()
    {
        compare_step(index, first_step, second_step, &step.compare)?;
    }
    Ok(())
}

fn compare_step(
    index: usize,
    first: &StepCapture,
    second: &StepCapture,
    compare: &StepCompare,
) -> Result<(), Difference> {
    if first.provider_total != second.provider_total {
        return Err(Difference {
            message: "provider request counts differ".to_owned(),
            pointer: format!("/steps/{index}/provider"),
            reference: Value::from(first.provider_total),
            rust: Value::from(second.provider_total),
        });
    }
    // a step's selector names a request by its index inside that step's own captures
    let bridge = Compare {
        headers: compare.headers.clone(),
        headers_present: compare.headers_present.clone(),
        upstream_headers: compare.upstream_headers.clone(),
        ignore: compare
            .ignore
            .iter()
            .filter(|ignored| ignored.pointer.starts_with("/upstream/"))
            .cloned()
            .collect(),
        contracts: None,
    };
    let reference = projected(first, compare);
    let rust = projected(second, compare);
    compare::compare(
        &reference,
        &rust,
        &first.provider,
        &second.provider,
        &bridge,
    )
    .map_err(|difference| mismatch(index, difference, first, second))
}

fn projected(capture: &StepCapture, compare: &StepCompare) -> Response {
    Response {
        status: capture.response.status,
        headers: capture.response.headers.clone(),
        body: error_projection(&capture.response.body, compare),
    }
}

fn error_projection(body: &Value, compare: &StepCompare) -> Value {
    let mut document = body.clone();
    for ignored in &compare.ignore {
        remove_pointer(&mut document, &ignored.pointer);
    }
    let mut projection = serde_json::Map::new();
    if let Some(Value::Object(fields)) = document.get("error") {
        let mut error = serde_json::Map::new();
        for field in ["type", "param", "code"] {
            if let Some(value) = fields.get(field) {
                error.insert(field.to_owned(), value.clone());
            }
        }
        projection.insert("error".to_owned(), Value::Object(error));
    }
    Value::Object(projection)
}

fn mismatch(
    index: usize,
    difference: ResponseDifference,
    first: &StepCapture,
    second: &StepCapture,
) -> Difference {
    Difference {
        message: difference.message,
        pointer: remap(index, &difference.pointer),
        reference: serde_json::to_value(side(first)).unwrap_or(Value::Null),
        rust: serde_json::to_value(side(second)).unwrap_or(Value::Null),
    }
}

fn remap(index: usize, pointer: &str) -> String {
    let mapped = pointer.strip_prefix("/upstream").map_or_else(
        || {
            pointer.strip_prefix("/headers").map_or_else(
                || match pointer {
                    "/status" => "/status".to_owned(),
                    "" | "/" => "/response".to_owned(),
                    other => format!("/response{other}"),
                },
                |rest| format!("/response/headers{rest}"),
            )
        },
        |rest| format!("/provider{rest}"),
    );
    format!("/steps/{index}{mapped}")
}
