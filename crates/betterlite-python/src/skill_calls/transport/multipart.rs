use betterlite_anthropic::skills::wire::CreateSkillRequest;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_models::decisions::Json;

use crate::skill_calls::convert::SkillFile;

const FILES_FIELD: &str = "files[]";

// The fields httpx writes first, then one part per file.
pub(super) fn parts(body: &CreateSkillRequest, files: Vec<SkillFile>) -> Vec<MultipartPart> {
    let mut parts = data_parts(body);
    parts.extend(files.into_iter().map(file_part));
    parts
}

fn data_parts(body: &CreateSkillRequest) -> Vec<MultipartPart> {
    let mut parts = Vec::new();
    if let Some(title) = body.display_title.as_deref() {
        parts.push(text_part("display_title", title));
    }
    for (name, value) in &body.extra {
        parts.push(text_part(name, &data_text(value)));
    }
    parts
}

fn text_part(name: &str, text: &str) -> MultipartPart {
    MultipartPart {
        name: name.to_owned(),
        file_name: None,
        content_type: None,
        data: text.as_bytes().to_vec(),
    }
}

fn data_text(value: &Json) -> String {
    match value {
        Json::Null => String::new(),
        Json::Bool(flag) => flag.to_string(),
        Json::Number(number) => number.to_string(),
        Json::Text(text) => text.clone(),
        Json::List(_) | Json::Object(_) => serde_json::to_string(value).unwrap_or_default(),
    }
}

fn file_part(file: SkillFile) -> MultipartPart {
    let guessed = file
        .content_type
        .or_else(|| Some(guess_content_type(&file.name)));
    MultipartPart {
        name: FILES_FIELD.to_owned(),
        file_name: Some(file.name),
        content_type: guessed,
        data: file.bytes,
    }
}

fn guess_content_type(name: &str) -> String {
    mime_guess::from_path(name)
        .first_raw()
        .unwrap_or("application/octet-stream")
        .to_owned()
}
