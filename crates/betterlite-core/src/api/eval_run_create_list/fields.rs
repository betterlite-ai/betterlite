use betterlite_openai::evals::types::{CreateRunRequest, EvalOrder, ListRunsParams};
use betterlite_openai::openai_base_config::Object;

use super::args::{CreateRunArgs, ListRunsArgs};
use super::error::EvalRunCallError;

// CreateRunRequest as LiteLLM builds it: data_source, name when given, then extra_body's own three keys
pub(super) fn create_request(
    request: CreateRunArgs<'_>,
) -> Result<CreateRunRequest, EvalRunCallError> {
    let Some(extra) = request.extra_body else {
        return Ok(CreateRunRequest {
            data_source: request.data_source.clone(),
            name: request.name.map(str::to_owned),
            metadata: None,
        });
    };
    reject_unknown(extra)?;
    Ok(CreateRunRequest {
        data_source: replaced_data_source(extra, request.data_source.clone())?,
        name: replaced_text("name", extra, request.name.map(str::to_owned))?,
        metadata: extra_metadata(extra)?,
    })
}

// create_request.update() in Python writes any key; CreateRunRequest declares three, so the rest are refused
fn reject_unknown(extra: &Object) -> Result<(), EvalRunCallError> {
    for (key, value) in extra {
        if matches!(key.as_str(), "data_source" | "name" | "metadata") || value.is_null() {
            continue;
        }
        return Err(EvalRunCallError::UnknownBodyKey { key: key.clone() });
    }
    Ok(())
}

// extra_body's data_source replaces the caller's, and CreateRunRequest declares it an object
fn replaced_data_source(extra: &Object, current: Object) -> Result<Object, EvalRunCallError> {
    let Some(value) = extra.get("data_source") else {
        return Ok(current);
    };
    match value.as_object() {
        Some(found) => Ok(found.clone()),
        None => Err(EvalRunCallError::FieldType {
            field: "data_source",
            expected: "an object",
        }),
    }
}

// CreateRunRequest.metadata starts absent; only extra_body sets it, and a null there clears it
fn extra_metadata(extra: &Object) -> Result<Option<Object>, EvalRunCallError> {
    let Some(value) = extra.get("metadata") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    match value.as_object() {
        Some(found) => Ok(Some(found.clone())),
        None => Err(EvalRunCallError::FieldType {
            field: "metadata",
            expected: "an object or null",
        }),
    }
}

// name, after and before are optional: an extra value replaces them, and a null clears them
fn replaced_text(
    field: &'static str,
    extra: &Object,
    current: Option<String>,
) -> Result<Option<String>, EvalRunCallError> {
    let Some(value) = extra.get(field) else {
        return Ok(current);
    };
    if value.is_null() {
        return Ok(None);
    }
    match value.as_str() {
        Some(found) => Ok(Some(found.to_owned())),
        None => Err(EvalRunCallError::FieldType {
            field,
            expected: "a string or null",
        }),
    }
}

// ListRunsParams.order is Literal["asc", "desc"], while LiteLLM's signature only declares str
fn order_of(order: &str) -> Result<EvalOrder, EvalRunCallError> {
    match order {
        "asc" => Ok(EvalOrder::Asc),
        "desc" => Ok(EvalOrder::Desc),
        _ => Err(EvalRunCallError::FieldType {
            field: "order",
            expected: "asc or desc",
        }),
    }
}

// only the explicit parameters are built first; extra_query's keys follow in LiteLLM and win
pub(super) fn list_params(request: ListRunsArgs<'_>) -> Result<ListRunsParams, EvalRunCallError> {
    let params = ListRunsParams {
        limit: request.limit,
        after: request.after.map(str::to_owned),
        before: request.before.map(str::to_owned),
        order: request.order.map(order_of).transpose()?,
    };
    match request.extra_query {
        Some(extra) => with_extra_query(params, extra),
        None => Ok(params),
    }
}

// LiteLLM's list_params.update(extra_query) overwrites the explicit values, null included
fn with_extra_query(
    mut params: ListRunsParams,
    extra: &Object,
) -> Result<ListRunsParams, EvalRunCallError> {
    if let Some(value) = extra.get("limit") {
        params.limit = if value.is_null() {
            None
        } else {
            Some(value.as_i64().ok_or_else(|| EvalRunCallError::FieldType {
                field: "limit",
                expected: "an integer or null",
            })?)
        };
    }
    params.after = replaced_text("after", extra, params.after)?;
    params.before = replaced_text("before", extra, params.before)?;
    if let Some(value) = extra.get("order") {
        let order = if value.is_null() {
            None
        } else {
            Some(value.as_str().ok_or_else(|| EvalRunCallError::FieldType {
                field: "order",
                expected: "asc, desc or null",
            })?)
        };
        params.order = order.map(order_of).transpose()?;
    }
    Ok(params)
}
