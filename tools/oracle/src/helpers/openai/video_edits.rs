use std::io::Cursor;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_video_identifiers::multipart::MediaBody;
use betterlite_openai::video_edits::character::{
    CharacterPart, CreateCharacter, transform_video_create_character_request,
    transform_video_get_character_request,
};
use betterlite_openai::video_edits::edit::{VideoEditRequest, transform_video_edit_request};
use betterlite_openai::video_edits::error::VideoEditError;
use betterlite_openai::video_edits::extension::{
    VideoExtensionRequest, transform_video_extension_request,
};
use betterlite_openai::video_edits::response::{
    VideoReply, decode_character_reply, transform_video_response,
};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Fixture {
    Bytes {
        data_base64: String,
    },
    Text {
        value: String,
    },
    Stream {
        data_base64: String,
        #[serde(default)]
        name: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum Call {
    CreateCharacterRequest {
        name: String,
        fixture: Fixture,
        api_base: String,
    },
    CreateCharacterResponse {
        body: String,
    },
    GetCharacterRequest {
        character_id: String,
        api_base: String,
    },
    GetCharacterResponse {
        body: String,
    },
    EditRequest {
        prompt: String,
        video_id: String,
        api_base: String,
        #[serde(default)]
        fixture: Option<Fixture>,
        #[serde(default)]
        extra_body: Option<IndexMap<String, Json>>,
        #[serde(default)]
        prefetched_source_data: Option<Json>,
    },
    EditResponse {
        body: String,
        #[serde(default)]
        provider: Option<String>,
        #[serde(default)]
        request_data: Option<Json>,
    },
    ExtensionRequest {
        prompt: String,
        video_id: String,
        seconds: String,
        api_base: String,
        #[serde(default)]
        extra_body: Option<IndexMap<String, Json>>,
    },
    ExtensionResponse {
        body: String,
        #[serde(default)]
        provider: Option<String>,
    },
}

enum PreparedKind {
    Bytes(Vec<u8>),
    Text(String),
    Reader(Cursor<Vec<u8>>),
}

struct Prepared {
    name: Option<String>,
    kind: PreparedKind,
}

impl Prepared {
    fn media(&mut self) -> MediaBody<'_> {
        match &mut self.kind {
            PreparedKind::Bytes(bytes) => MediaBody::Bytes(bytes.as_slice()),
            PreparedKind::Text(text) => MediaBody::Text(text.as_str()),
            PreparedKind::Reader(reader) => MediaBody::Stream {
                name: self.name.as_deref(),
                reader,
            },
        }
    }

    fn body(&self) -> Value {
        match &self.kind {
            PreparedKind::Bytes(bytes) => Value::String(STANDARD.encode(bytes)),
            PreparedKind::Text(text) => Value::String(text.clone()),
            PreparedKind::Reader(_) => Value::Null,
        }
    }

    fn body_kind(&self) -> &'static str {
        match &self.kind {
            PreparedKind::Bytes(_) => "bytes",
            PreparedKind::Text(_) => "text",
            PreparedKind::Reader(_) => "stream",
        }
    }
}

fn prepared(fixture: &Fixture) -> Prepared {
    match fixture {
        Fixture::Bytes { data_base64 } => Prepared {
            name: None,
            kind: PreparedKind::Bytes(STANDARD.decode(data_base64).unwrap_or_default()),
        },
        Fixture::Text { value } => Prepared {
            name: None,
            kind: PreparedKind::Text(value.clone()),
        },
        Fixture::Stream { data_base64, name } => Prepared {
            name: name.clone(),
            kind: PreparedKind::Reader(Cursor::new(
                STANDARD.decode(data_base64).unwrap_or_default(),
            )),
        },
    }
}

fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn failure(error: &VideoEditError) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("VideoEditError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn part_value(
    field: &str,
    filename: Option<&str>,
    content_type: Option<&str>,
    kind: &str,
    body: &Value,
) -> Value {
    json!({
        "field": field,
        "filename": filename,
        "content_type": content_type,
        "body_kind": kind,
        "body": body,
    })
}

fn character_files(created: &CreateCharacter<'_>, kind: &str, body: &Value) -> Value {
    Value::Array(
        created
            .files
            .iter()
            .map(|part| match part {
                CharacterPart::Text { field, text } => {
                    part_value(*field, None, None, "text", &json!(*text))
                }
                CharacterPart::File(file) => part_value(
                    file.field.as_str(),
                    Some(file.filename.as_str()),
                    Some(file.content_type),
                    kind,
                    body,
                ),
            })
            .collect(),
    )
}

fn create_character_value(call: &Call) -> HelperResult {
    let Call::CreateCharacterRequest {
        name,
        fixture,
        api_base,
    } = call
    else {
        return Err(invalid(String::from("not a create character request")));
    };
    let mut state = prepared(fixture);
    let kind = state.body_kind();
    let body = state.body();
    let created = transform_video_create_character_request(name, state.media(), api_base);
    let files = character_files(&created, kind, &body);
    Ok(json!({"url": created.url, "files": files}))
}

fn get_character_value(call: &Call) -> HelperResult {
    let Call::GetCharacterRequest {
        character_id,
        api_base,
    } = call
    else {
        return Err(invalid(String::from("not a get character request")));
    };
    let url = transform_video_get_character_request(character_id, api_base)
        .map_err(|error| failure(&error))?;
    Ok(json!({"url": url}))
}

fn edit_request_value(call: &Call) -> HelperResult {
    let Call::EditRequest {
        prompt,
        video_id,
        api_base,
        fixture,
        extra_body,
        prefetched_source_data,
    } = call
    else {
        return Err(invalid(String::from("not an edit request")));
    };
    let mut state = fixture.as_ref().map(prepared);
    let (kind, body, video_file) = match state.as_mut() {
        Some(prepared) => (
            prepared.body_kind(),
            prepared.body(),
            Some(prepared.media()),
        ),
        None => ("", Value::Null, None),
    };
    let edit = transform_video_edit_request(VideoEditRequest {
        prompt: prompt.clone(),
        video_id: video_id.clone(),
        api_base: api_base.clone(),
        video_file,
        extra_body: extra_body.clone(),
        prefetched_source_data: prefetched_source_data.clone(),
    });
    let files = match &edit.files {
        Some(file) => json!([part_value(
            file.field.as_str(),
            Some(file.filename.as_str()),
            Some(file.content_type),
            kind,
            &body,
        )]),
        None => Value::Null,
    };
    Ok(json!({"url": edit.url, "body": edit.body, "files": files}))
}

fn extension_request_value(call: &Call) -> HelperResult {
    let Call::ExtensionRequest {
        prompt,
        video_id,
        seconds,
        api_base,
        extra_body,
    } = call
    else {
        return Err(invalid(String::from("not an extension request")));
    };
    let extension = transform_video_extension_request(VideoExtensionRequest {
        prompt: prompt.clone(),
        video_id: video_id.clone(),
        seconds: seconds.clone(),
        api_base: api_base.clone(),
        extra_body: extra_body.clone(),
    });
    Ok(json!({"url": extension.url, "body": extension.body}))
}

fn create_character_reply(call: &Call) -> HelperResult {
    let Call::CreateCharacterResponse { body } = call else {
        return Err(invalid(String::from("not a create character response")));
    };
    let object = decode_character_reply(body.as_bytes()).map_err(|error| failure(&error))?;
    serde_json::to_value(&object).map_err(|error| invalid(error.to_string()))
}

fn get_character_reply(call: &Call) -> HelperResult {
    let Call::GetCharacterResponse { body } = call else {
        return Err(invalid(String::from("not a get character response")));
    };
    let object = decode_character_reply(body.as_bytes()).map_err(|error| failure(&error))?;
    serde_json::to_value(&object).map_err(|error| invalid(error.to_string()))
}

fn video_reply_value(
    body: &str,
    provider: Option<&str>,
    request_data: Option<&Json>,
) -> HelperResult {
    let reply = VideoReply {
        body: body.as_bytes(),
        custom_llm_provider: provider,
        request_data,
    };
    let object = transform_video_response(&reply).map_err(|error| failure(&error))?;
    serde_json::to_value(&object).map_err(|error| invalid(error.to_string()))
}

fn edit_reply_value(call: &Call) -> HelperResult {
    let Call::EditResponse {
        body,
        provider,
        request_data,
    } = call
    else {
        return Err(invalid(String::from("not an edit response")));
    };
    video_reply_value(body, provider.as_deref(), request_data.as_ref())
}

fn extension_reply_value(call: &Call) -> HelperResult {
    let Call::ExtensionResponse { body, provider } = call else {
        return Err(invalid(String::from("not an extension response")));
    };
    video_reply_value(body, provider.as_deref(), None)
}

fn run_call(call: &Call) -> HelperResult {
    match call {
        Call::CreateCharacterRequest { .. } => create_character_value(call),
        Call::CreateCharacterResponse { .. } => create_character_reply(call),
        Call::GetCharacterRequest { .. } => get_character_value(call),
        Call::GetCharacterResponse { .. } => get_character_reply(call),
        Call::EditRequest { .. } => edit_request_value(call),
        Call::EditResponse { .. } => edit_reply_value(call),
        Call::ExtensionRequest { .. } => extension_request_value(call),
        Call::ExtensionResponse { .. } => extension_reply_value(call),
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut results = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            results.push(run_call(call)?);
        }
        Ok(Value::Array(results))
    }
}
