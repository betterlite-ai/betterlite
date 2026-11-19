use betterlite_models::decisions::Json;

// B192's bounded reader limits, carried here because betterlite-logging depends on this crate: a request_tags payload stays under 1 MiB and 64 containers of nesting.
pub const MAX_ENQUEUE_TAGS_BYTES: usize = 0x0010_0000;
pub const MAX_ENQUEUE_TAGS_DEPTH: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum EnqueueTagsError {
    #[error("request_tags payload is {len} bytes, over the {MAX_ENQUEUE_TAGS_BYTES} byte limit")]
    TooLarge { len: usize },
    #[error("request_tags payload nests deeper than {MAX_ENQUEUE_TAGS_DEPTH} containers")]
    TooDeep,
    #[error("request_tags payload is a truthy scalar that does not iterate")]
    NotIterable,
}

// The two shapes _update_tag_db accepts: the serialized request_tags column and an already parsed tag list.
#[derive(Clone, Copy, Debug)]
pub enum RequestTags<'text> {
    Serialized(&'text str),
    Parsed(&'text [String]),
}

// safe_json_loads(request_tags, default=[]) plus the iteration _update_tag_db performs: list members, object keys or a string's characters.
pub fn decode_enqueue_tags(bytes: &[u8]) -> Result<Vec<String>, EnqueueTagsError> {
    if bytes.len() > MAX_ENQUEUE_TAGS_BYTES {
        return Err(EnqueueTagsError::TooLarge { len: bytes.len() });
    }
    let Ok(document) = serde_json::from_slice::<Json>(bytes) else {
        return Ok(Vec::new());
    };
    if container_depth(&document) > MAX_ENQUEUE_TAGS_DEPTH {
        return Err(EnqueueTagsError::TooDeep);
    }
    match &document {
        Json::List(items) => Ok(items
            .iter()
            .filter_map(|item| match item {
                Json::Text(text) => Some(text.as_str()),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                    None
                }
            })
            .map(str::to_owned)
            .collect()),
        Json::Text(text) => Ok(text.chars().map(String::from).collect()),
        Json::Object(fields) => Ok(fields.keys().cloned().collect()),
        Json::Null => Ok(Vec::new()),
        Json::Bool(flag) if !flag => Ok(Vec::new()),
        // a zero number is falsey; every other number is truthy and, unlike a list or object, does not iterate
        Json::Number(number) if number.as_f64().is_some_and(|value| value == 0.0_f64) => {
            Ok(Vec::new())
        }
        Json::Bool(_) | Json::Number(_) => Err(EnqueueTagsError::NotIterable),
    }
}

// Counts container nesting only, so a scalar at the innermost level does not push a document over the bound.
fn container_depth(document: &Json) -> usize {
    let mut pending = vec![(document, 0_usize)];
    let mut deepest = 0;
    while let Some((node, depth)) = pending.pop() {
        let next = depth.saturating_add(1);
        match node {
            Json::List(items) => {
                pending.extend(items.iter().map(|item| (item, next)));
                deepest = deepest.max(next);
            }
            Json::Object(fields) => {
                pending.extend(fields.values().map(|field| (field, next)));
                deepest = deepest.max(next);
            }
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {}
        }
    }
    deepest
}
