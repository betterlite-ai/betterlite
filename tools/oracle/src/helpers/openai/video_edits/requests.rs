use betterlite_openai::video_edits::character::{
    // ast-grep-ignore: no-long-names
    transform_video_create_character_request,
    // ast-grep-ignore: no-long-names
    transform_video_get_character_request,
};
use betterlite_openai::video_edits::edit::{VideoEditRequest, transform_video_edit_request};
use betterlite_openai::video_edits::extension::{
    VideoExtensionRequest,
    // ast-grep-ignore: no-long-names
    transform_video_extension_request,
};
use serde_json::{Value, json};

use super::fixture::{Prepared, character_files, file_part_value, prepared};
use super::support::helper_error;
use super::{CreateCharacterRequest, EditRequest, ExtensionRequest, GetCharacterRequest};
use crate::helpers::HelperResult;

pub(super) fn create_character_value(request: &CreateCharacterRequest) -> HelperResult {
    let mut state = prepared(&request.fixture)?;
    let api_base = request.api_base.as_str();
    // ast-grep-ignore: no-long-names
    let mut created =
        transform_video_create_character_request(&request.name, state.media(), api_base);
    let files = character_files(&mut created)?;
    Ok(json!({"url": created.url, "files": files}))
}

pub(super) fn get_character_value(request: &GetCharacterRequest) -> HelperResult {
    // ast-grep-ignore: no-long-names
    let url = transform_video_get_character_request(&request.character_id, &request.api_base)
        .map_err(|error| helper_error("VideoEditError", error.to_string()))?;
    Ok(json!({"url": url}))
}

pub(super) fn edit_request_value(request: &EditRequest) -> HelperResult {
    let mut state = match &request.fixture {
        Some(fixture) => Some(prepared(fixture)?),
        None => None,
    };
    let mut edit = transform_video_edit_request(VideoEditRequest {
        prompt: request.prompt.clone(),
        video_id: request.video_id.clone(),
        api_base: request.api_base.clone(),
        video_file: state.as_mut().map(Prepared::media),
        extra_body: request.extra_body.clone(),
        prefetched_source_data: request.prefetched_source_data.clone(),
    });
    let files = match edit.files.as_mut() {
        Some(file) => json!([file_part_value(file)?]),
        None => Value::Null,
    };
    Ok(json!({"url": edit.url, "body": edit.body, "files": files}))
}

pub(super) fn extension_request_value(request: &ExtensionRequest) -> HelperResult {
    // ast-grep-ignore: no-long-names
    let extension = transform_video_extension_request(VideoExtensionRequest {
        prompt: request.prompt.clone(),
        video_id: request.video_id.clone(),
        seconds: request.seconds.clone(),
        api_base: request.api_base.clone(),
        extra_body: request.extra_body.clone(),
    });
    Ok(json!({"url": extension.url, "body": extension.body}))
}
