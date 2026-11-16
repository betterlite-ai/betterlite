use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::event_types::Extra;
use betterlite_openai::api::responses_shapes::OutputAnnotation;
use indexmap::IndexMap;
use serde_json::Number;

use crate::api::chat_response_stream_tools::fields::{list, object};
use crate::api::responses_output_details::{ChatAnnotation, citation_fields};

use super::error::StreamItemsError;

pub(super) fn completion_content(completion: &Json) -> Result<&str, StreamItemsError> {
    let Some(fields) = message_fields(completion)? else {
        return Ok("");
    };
    match fields.get("content") {
        None | Some(Json::Null) => Ok(""),
        Some(Json::Text(text)) => Ok(text),
        Some(_) => Err(StreamItemsError::ContentNotText),
    }
}

pub(super) fn completion_annotations(completion: &Json) -> Result<Vec<Extra>, StreamItemsError> {
    let Some(fields) = message_fields(completion)? else {
        return Ok(citation_extras(None));
    };
    let items = match fields.get("annotations") {
        None | Some(Json::Null) => return Ok(citation_extras(None)),
        Some(Json::List(items)) => items,
        Some(_) => return Err(StreamItemsError::AnnotationsNotList),
    };
    let mut parsed = Vec::with_capacity(items.len());
    for item in items {
        parsed.push(parse_annotation(item)?);
    }
    Ok(citation_extras(Some(&parsed)))
}

fn message_fields(completion: &Json) -> Result<Option<&IndexMap<String, Json>>, StreamItemsError> {
    let choice = first_choice(completion)?;
    let message = object(choice).and_then(|fields| fields.get("message"));
    Ok(message.and_then(object))
}

fn first_choice(completion: &Json) -> Result<&Json, StreamItemsError> {
    object(completion)
        .and_then(|fields| fields.get("choices"))
        .and_then(list)
        .and_then(|choices| choices.first())
        .ok_or(StreamItemsError::MissingChoice)
}

fn parse_annotation(item: &Json) -> Result<ChatAnnotation, StreamItemsError> {
    let encoded =
        serde_json::to_vec(item).map_err(|source| StreamItemsError::Annotation { source })?;
    serde_json::from_slice(&encoded).map_err(|source| StreamItemsError::Annotation { source })
}

fn citation_extras(annotations: Option<&[ChatAnnotation]>) -> Vec<Extra> {
    citation_fields(annotations)
        .iter()
        .map(annotation_extra)
        .collect()
}

fn annotation_extra(annotation: &OutputAnnotation) -> Extra {
    let text_json =
        |value: Option<&str>| value.map_or(Json::Null, |text| Json::Text(text.to_owned()));
    let number_json =
        |value: Option<i64>| value.map_or(Json::Null, |number| Json::Number(Number::from(number)));
    let mut extra = Extra::from([
        ("type".to_owned(), text_json(annotation.r#type.as_deref())),
        (
            "start_index".to_owned(),
            number_json(annotation.start_index),
        ),
        ("end_index".to_owned(), number_json(annotation.end_index)),
        ("url".to_owned(), text_json(annotation.url.as_deref())),
        ("title".to_owned(), text_json(annotation.title.as_deref())),
    ]);
    for (key, value) in &annotation.extra {
        extra.insert(key.clone(), value.clone());
    }
    extra
}
