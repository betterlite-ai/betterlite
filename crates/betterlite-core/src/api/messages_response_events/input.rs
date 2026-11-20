use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponseApiUsage;
use indexmap::IndexMap;

use crate::api::messages_options::refusal::OutputItem;
use crate::api::responses_stream_primitives::error_fields::StreamErrorAttributes;

// An empty replacement for a delta the event never carried, the value getattr's "" default becomes for a dict event.
static EMPTY_DELTA: Json = Json::Text(String::new());

// One parsed Responses event, as B1113 hands it over: the attributes getattr reads, and the decoded mapping a dict or dict subclass also has.
#[derive(Clone, Copy)]
pub struct ParsedEvent<'event> {
    pub attributes: EventAttributes<'event>,
    pub mapping: Option<&'event IndexMap<String, Json>>,
}

// The attributes getattr reads off a provider event object; an absent field stays None as getattr's default does.
#[derive(Clone, Copy, Default)]
pub struct EventAttributes<'event> {
    pub event_type: Option<&'event Json>,
    pub item: Option<&'event Json>,
    pub item_id: Option<&'event Json>,
    pub delta: Option<&'event Json>,
    pub summary_index: Option<&'event Json>,
    pub response: Option<ResponseAttribute<'event>>,
}

// The response of a completed or failed event: an object read by attribute, or a plain value that has no readable fields.
#[derive(Clone, Copy)]
pub enum ResponseAttribute<'event> {
    Object(ResponseAttributes<'event>),
    Value(&'event Json),
}

// A completed or failed event's response object, whose output, status, usage and error are read by attribute only.
#[derive(Clone, Copy)]
pub struct ResponseAttributes<'event> {
    pub output: &'event [OutputItem<'event>],
    pub status: Option<&'event str>,
    pub usage: Option<&'event ResponseApiUsage>,
    pub error: Option<ErrorAttribute<'event>>,
}

// The error of a failed response keeps the shape that produced it: a dict is read by key and a provider object by attribute.
#[derive(Clone, Copy)]
pub enum ErrorAttribute<'event> {
    Mapping(&'event IndexMap<String, Json>),
    Attributes(StreamErrorAttributes<'event>),
}

// An output item of an event: a decoded mapping or a provider object, which _field and the truthy-or-dict reads answer alike by name.
#[derive(Clone, Copy)]
pub struct ItemObject<'item> {
    pub(super) fields: Option<&'item IndexMap<String, Json>>,
}

impl<'event> ParsedEvent<'event> {
    // event.type falls back to the dict only when the attribute is null; a falsey "" attribute wins over the dict.
    #[must_use]
    pub(super) fn event_type(self) -> Option<&'event str> {
        let attribute = match self.attributes.event_type {
            Some(Json::Null) | None => self.mapping.and_then(|fields| fields.get("type")),
            Some(value) => Some(value),
        };
        attribute.and_then(text)
    }

    #[must_use]
    pub(super) fn item(self) -> Option<ItemObject<'event>> {
        let attribute = self.attributes.item.filter(|value| is_present(value));
        if let Some(value) = attribute {
            return Some(match value {
                Json::Object(fields) => ItemObject {
                    fields: Some(fields),
                },
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                    ItemObject { fields: None }
                }
            });
        }
        match self.mapping.and_then(|fields| fields.get("item")) {
            Some(Json::Object(fields)) => Some(ItemObject {
                fields: Some(fields),
            }),
            Some(Json::Null) | None => None,
            Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
                Some(ItemObject { fields: None })
            }
        }
    }

    // A truthy attribute item_id wins over the dict value; a falsey one falls back to it.
    #[must_use]
    pub(super) fn item_id(self) -> Option<&'event Json> {
        self.attributes
            .item_id
            .filter(|value| is_present(value))
            .or_else(|| self.mapping?.get("item_id"))
    }

    // A falsey attribute delta falls back to the dict value, and a non-dict event lands on getattr's "" default.
    #[must_use]
    pub(super) fn delta(self) -> &'event Json {
        self.attributes
            .delta
            .filter(|value| is_present(value))
            .unwrap_or_else(|| self.field("delta").unwrap_or(&EMPTY_DELTA))
    }

    // _field is shape-sensitive: a dict is read with get, and a provider object with getattr only.
    #[must_use]
    pub(super) fn field(self, name: &str) -> Option<&'event Json> {
        if let Some(fields) = self.mapping {
            return fields.get(name);
        }
        match name {
            "item_id" => self.attributes.item_id,
            "summary_index" => self.attributes.summary_index,
            _ => None,
        }
    }

    // The completed branch reads output, status and usage by attribute only, so a dict, scalar or absent response exposes no fields.
    #[must_use]
    pub(super) const fn response(self) -> Option<ResponseAttributes<'event>> {
        match self.attributes.response {
            Some(ResponseAttribute::Object(attributes)) => Some(attributes),
            Some(ResponseAttribute::Value(_)) | None => None,
        }
    }

    // _FailedResponseEvent.model_validate reads a dict event's response key and a provider object's attribute.
    pub(super) fn failed_error(
        self,
    ) -> Result<Option<ErrorAttribute<'event>>, InvalidFailedResponse> {
        let Some(fields) = self.mapping else {
            return match self.attributes.response {
                Some(ResponseAttribute::Object(attributes)) => Ok(attributes.error),
                Some(ResponseAttribute::Value(Json::Null)) | None => Ok(None),
                Some(ResponseAttribute::Value(Json::Object(response))) => {
                    Ok(response.get("error").and_then(error_object))
                }
                Some(ResponseAttribute::Value(_)) => Err(InvalidFailedResponse),
            };
        };
        match fields.get("response") {
            None | Some(Json::Null) => Ok(None),
            Some(Json::Object(response)) => Ok(response.get("error").and_then(error_object)),
            Some(_) => Err(InvalidFailedResponse),
        }
    }
}

// A dict error is read by key; any other value has no readable fields, as getattr finds none on a scalar or a list.
fn error_object(value: &Json) -> Option<ErrorAttribute<'_>> {
    match value {
        Json::Null => None,
        Json::Object(fields) => Some(ErrorAttribute::Mapping(fields)),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Some(ErrorAttribute::Attributes(StreamErrorAttributes::default()))
        }
    }
}

// A response pydantic refuses to read as a failed response, as a scalar or a list is.
#[derive(Debug)]
pub(super) struct InvalidFailedResponse;

#[must_use]
pub(super) const fn text(value: &Json) -> Option<&str> {
    match value {
        Json::Text(text) => Some(text.as_str()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

// Python's notion of presence, which the source applies to item ids, deltas and attributes before a fallback.
#[must_use]
pub(super) fn is_present(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => number
            .as_f64()
            .is_some_and(|candidate| candidate != 0.0_f64),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}
