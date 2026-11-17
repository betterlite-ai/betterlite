use betterlite_models::decisions::{Extra, Json};

const REQUESTS: &str = "requests";
const CONTENT: &str = "content";
const PARTS: &str = "parts";
const TEXT: &str = "text";

// The joined text LiteLLM hands the embedding reply as its input: every part of every request, one space apart.
pub(super) fn extract_input(request_body: Option<&Json>, batch: bool) -> String {
    let Some(Json::Object(body)) = request_body else {
        return String::new();
    };
    if batch {
        return batch_text(body);
    }
    parts_text(body)
}

fn batch_text(body: &Extra) -> String {
    let Some(Json::List(requests)) = body.get(REQUESTS) else {
        return String::new();
    };
    let mut texts = Vec::new();
    for request in requests {
        collect_parts(request, &mut texts);
    }
    texts.join(" ")
}

fn collect_parts(request: &Json, texts: &mut Vec<String>) {
    let Json::Object(fields) = request else {
        return;
    };
    let Some(parts) = fields.get(CONTENT).and_then(parts_list) else {
        return;
    };
    texts.extend(parts.iter().map(part_text));
}

fn parts_text(body: &Extra) -> String {
    let Some(parts) = body.get(CONTENT).and_then(parts_list) else {
        return String::new();
    };
    parts.iter().map(part_text).collect::<Vec<_>>().join(" ")
}

fn parts_list(content: &Json) -> Option<&[Json]> {
    let Json::Object(fields) = content else {
        return None;
    };
    let Some(value) = fields.get(PARTS) else {
        return None;
    };
    let Json::List(parts) = value else {
        return None;
    };
    Some(parts.as_slice())
}

// A part with no text, or a text of another type, joins as an empty string.
fn part_text(part: &Json) -> String {
    let Json::Object(fields) = part else {
        return String::new();
    };
    let Some(value) = fields.get(TEXT) else {
        return String::new();
    };
    let Json::Text(text) = value else {
        return String::new();
    };
    text.clone()
}
