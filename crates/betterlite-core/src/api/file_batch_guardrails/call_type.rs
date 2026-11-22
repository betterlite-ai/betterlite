use betterlite_models::provider_defaults_a::url::urlsplit;

use super::record_json::text::RecordText;
use super::record_json::{RecordJson, RecordObject};
use super::{BODY_SHAPE_CALL_TYPES, SCANNABLE_CALL_TYPES};
use crate::routes::CallType;
use crate::routes::catalog::RouteCatalog;

// An absolute url reduces to its path, or a record naming /v1/responses in full would be scanned as an embedding
#[must_use]
pub fn call_type_from_url(routes: &RouteCatalog, url: &str) -> Option<CallType> {
    let path = urlsplit(url).ok()?.path;
    // a path past the catalog's limits is one no route names, which the body shape already handles
    let call_types = routes
        .call_types_for(path.trim_end_matches('/'))
        .ok()
        .flatten()?;
    call_types
        .iter()
        .copied()
        .find(|call_type| SCANNABLE_CALL_TYPES.contains(call_type))
}

#[must_use]
pub fn call_type_from_body(body: &RecordObject) -> Option<CallType> {
    BODY_SHAPE_CALL_TYPES
        .iter()
        .find(|(field, _)| body.contains_key(&RecordText::from(*field)))
        .map(|(_, call_type)| *call_type)
}

// An unrecognized url falls back to the body, since provider transformers read an unknown url as chat
#[must_use]
pub fn scannable_call_type(
    routes: &RouteCatalog,
    url: Option<&RecordJson>,
    body: &RecordObject,
) -> Option<CallType> {
    // no route has a surrogate, so a stand-in that is not a url delimiter keeps every match as it was
    let from_url = url
        .and_then(RecordJson::as_text)
        .map(|text| text.replaced('\u{fffd}'))
        .filter(|text| !text.is_empty())
        .and_then(|text| call_type_from_url(routes, &text));
    from_url.or_else(|| call_type_from_body(body))
}
