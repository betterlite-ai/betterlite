use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::event_types::{Extra, ResponseEvent};
use betterlite_openai::api::responses_event_records::records::{
    KnownResponseEvent, ResponsesAPIStreamingResponse,
};
use betterlite_openai::api::responses_objects::ResponsesApiResponse;

use super::ADVERTISED_ID_PREFIX;
use super::error::Error;

pub(super) type Rewrite<'call> = &'call mut dyn FnMut(&str) -> Result<String, Error>;

pub(super) fn is_create_route(request_route: Option<&str>) -> bool {
    let Some(route) = request_route else {
        return false;
    };
    // LiteLLM strips the provider prefix by length, so the canonical route keeps its own leading slash
    let canonical = if route.starts_with("/openai/") {
        route.get("/openai".len()..).unwrap_or(route)
    } else {
        route
    };
    matches!(canonical, "/v1/responses" | "/responses")
}

// the nonstream hook rewrites any ResponsesAPIResponse, whatever route produced it
pub(super) fn rewrite_response(
    response: &mut ResponsesApiResponse,
    rewrite: Rewrite<'_>,
) -> Result<(), Error> {
    if !response.id.starts_with(ADVERTISED_ID_PREFIX) {
        return rewrite_nested_mapping(&mut response.extra, rewrite);
    }
    response.id = rewrite(&response.id)?;
    Ok(())
}

pub(super) fn rewrite_event(
    event: &mut ResponsesAPIStreamingResponse,
    rewrite: Rewrite<'_>,
) -> Result<(), Error> {
    match event {
        ResponsesAPIStreamingResponse::Known(known) => rewrite_known(known, rewrite),
        ResponsesAPIStreamingResponse::Generic(generic) => {
            rewrite_extra(&mut generic.extra, rewrite)
        }
    }
}

fn rewrite_known(event: &mut KnownResponseEvent, rewrite: Rewrite<'_>) -> Result<(), Error> {
    match event {
        KnownResponseEvent::ResponseCreated(record)
        | KnownResponseEvent::ResponseInProgress(record)
        | KnownResponseEvent::ResponseCompleted(record)
        | KnownResponseEvent::ResponseFailed(record)
        | KnownResponseEvent::ResponseIncomplete(record) => rewrite_lifecycle(record, rewrite),
        KnownResponseEvent::ResponsePartAdded(record) => rewrite_extra(&mut record.extra, rewrite),
        KnownResponseEvent::ReasoningSummaryTextDelta(record) => {
            rewrite_extra(&mut record.extra, rewrite)
        }
        KnownResponseEvent::ReasoningSummaryTextDone(record) => {
            rewrite_extra(&mut record.extra, rewrite)
        }
        KnownResponseEvent::ReasoningSummaryPartDone(record) => {
            rewrite_extra(&mut record.extra, rewrite)
        }
        KnownResponseEvent::OutputItemAdded(record) => rewrite_extra(&mut record.extra, rewrite),
        KnownResponseEvent::OutputItemDone(record) => rewrite_extra(&mut record.extra, rewrite),
        KnownResponseEvent::ContentPartAdded(record) => rewrite_extra(&mut record.extra, rewrite),
        KnownResponseEvent::ContentPartDone(record) => rewrite_extra(&mut record.extra, rewrite),
        KnownResponseEvent::OutputTextDelta(record) => rewrite_extra(&mut record.extra, rewrite),
        KnownResponseEvent::OutputTextAnnotationAdded(record) => {
            rewrite_extra(&mut record.extra, rewrite)
        }
        KnownResponseEvent::OutputTextDone(record) => rewrite_extra(&mut record.extra, rewrite),
        KnownResponseEvent::RefusalDelta(record) => rewrite_extra(&mut record.extra, rewrite),
        KnownResponseEvent::RefusalDone(record) => rewrite_extra(&mut record.extra, rewrite),
        KnownResponseEvent::FunctionCallArgumentsDelta(record) => {
            rewrite_extra(&mut record.extra, rewrite)
        }
        KnownResponseEvent::FunctionCallArgumentsDone(record) => {
            rewrite_extra(&mut record.extra, rewrite)
        }
        KnownResponseEvent::FileSearchCallInProgress(record)
        | KnownResponseEvent::FileSearchCallSearching(record)
        | KnownResponseEvent::FileSearchCallCompleted(record)
        | KnownResponseEvent::WebSearchCallInProgress(record)
        | KnownResponseEvent::WebSearchCallSearching(record)
        | KnownResponseEvent::WebSearchCallCompleted(record) => {
            rewrite_extra(&mut record.extra, rewrite)
        }
        KnownResponseEvent::McpListToolsInProgress(record)
        | KnownResponseEvent::McpListToolsCompleted(record)
        | KnownResponseEvent::McpListToolsFailed(record)
        | KnownResponseEvent::McpCallInProgress(record) => {
            rewrite_extra(&mut record.extra, rewrite)
        }
        KnownResponseEvent::McpCallArgumentsDelta(record) => {
            rewrite_extra(&mut record.extra, rewrite)
        }
        KnownResponseEvent::McpCallArgumentsDone(record) => {
            rewrite_extra(&mut record.extra, rewrite)
        }
        KnownResponseEvent::McpCallCompleted(record)
        | KnownResponseEvent::McpCallFailed(record) => rewrite_extra(&mut record.extra, rewrite),
        KnownResponseEvent::ImageGenerationPartialImage(record) => {
            rewrite_extra(&mut record.extra, rewrite)
        }
        KnownResponseEvent::Error(record) => rewrite_extra(&mut record.extra, rewrite),
    }
}

// getattr(event, "response") on a lifecycle event is its typed response, rewritten without a prefix check
fn rewrite_lifecycle(record: &mut ResponseEvent, rewrite: Rewrite<'_>) -> Result<(), Error> {
    if rewrite_top_level_id(&mut record.extra, rewrite)? {
        return Ok(());
    }
    record.response.id = rewrite(&record.response.id)?;
    Ok(())
}

// a pydantic event model answers getattr("response") from its allowed extras
fn rewrite_extra(extra: &mut Extra, rewrite: Rewrite<'_>) -> Result<(), Error> {
    if rewrite_top_level_id(extra, rewrite)? {
        return Ok(());
    }
    rewrite_nested_mapping(extra, rewrite)
}

fn rewrite_top_level_id(extra: &mut Extra, rewrite: Rewrite<'_>) -> Result<bool, Error> {
    let Some(Json::Text(id)) = extra.get("id") else {
        return Ok(false);
    };
    if !id.starts_with(ADVERTISED_ID_PREFIX) {
        return Ok(false);
    }
    let rewritten = rewrite(id)?;
    extra.insert("id".to_owned(), Json::Text(rewritten));
    Ok(true)
}

// a nested pydantic model or mapping whose id is a string is rewritten, without a prefix check
fn rewrite_nested_mapping(extra: &mut Extra, rewrite: Rewrite<'_>) -> Result<(), Error> {
    let Some(Json::Object(fields)) = extra.get_mut("response") else {
        return Ok(());
    };
    let Some(Json::Text(id)) = fields.get("id") else {
        return Ok(());
    };
    let rewritten = rewrite(id)?;
    fields.insert("id".to_owned(), Json::Text(rewritten));
    Ok(())
}
