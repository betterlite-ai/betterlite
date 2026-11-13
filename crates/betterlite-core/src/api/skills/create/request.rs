use betterlite_anthropic::skills::wire::CreateSkillRequest;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::CreateSkillError;

// LiteLLM builds create_request from display_title and files, then overlays every extra_body key
pub fn assembled_request(
    display_title: Option<&str>,
    files: Option<&[Json]>,
    extra_body: Option<&IndexMap<String, Json>>,
) -> Result<CreateSkillRequest, CreateSkillError> {
    let mut request = CreateSkillRequest {
        display_title: display_title.map(str::to_owned),
        files: files.map(<[Json]>::to_vec),
        extra: IndexMap::new(),
    };
    let Some(body) = extra_body else {
        return Ok(request);
    };
    for (name, value) in body {
        match (name.as_str(), value) {
            ("display_title", Json::Null) => request.display_title = None,
            ("display_title", Json::Text(title)) => request.display_title = Some(title.clone()),
            (
                "display_title",
                Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            ) => {
                return Err(CreateSkillError::FieldType {
                    name: "display_title",
                });
            }
            ("files", Json::Null) => request.files = None,
            ("files", Json::List(items)) => request.files = Some(items.clone()),
            ("files", Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
                return Err(CreateSkillError::FieldType { name: "files" });
            }
            (_, other) => {
                request.extra.insert(name.clone(), other.clone());
            }
        }
    }
    Ok(request)
}

// a kwargs or extra_body value that LiteLLM's str | None declarations accept
pub fn text_field<'values>(
    values: &'values IndexMap<String, Json>,
    name: &'static str,
) -> Result<Option<&'values str>, CreateSkillError> {
    match values.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text)),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(CreateSkillError::FieldType { name })
        }
    }
}
