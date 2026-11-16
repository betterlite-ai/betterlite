mod scripted;
mod steps;

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_context::dynamic_logging_cache::Clock;
use betterlite_core::skills::operations::{
    HostedSkillCreation, HostedSkillOps, SkillUpload, SkillUploadType,
};
use betterlite_core::skills::store::{HostedSkillStore, SkillIdSource};
use betterlite_db::managed_resource_rows::skills::SkillTable;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

use crate::helpers::{Helper, HelperError, HelperResult};

use scripted::Scripted;
use steps::run_steps;

const CACHE_CLOCK: f64 = 1000.0;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    now: String,
    uuid: String,
    #[serde(default)]
    rows: Vec<SkillTable>,
    #[serde(default)]
    page: Vec<SkillTable>,
    steps: Vec<Step>,
}

// LiteLLM's create call takes files as (name, content) or (name, content, content_type) tuples
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Step {
    action: String,
    display_title: Option<String>,
    description: Option<String>,
    instructions: Option<String>,
    files: Option<Vec<Vec<Json>>>,
    file_content: Option<String>,
    file_name: Option<String>,
    file_type: Option<String>,
    metadata: Option<IndexMap<String, Json>>,
    user_id: Option<String>,
    principal: Option<PrincipalFields>,
    skill_id: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
    litellm_call_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PrincipalFields {
    user_id: Option<String>,
    team_id: Option<String>,
    org_id: Option<String>,
    api_key: Option<String>,
    token: Option<String>,
    #[serde(default)]
    admin: bool,
}

pub(crate) struct Op;

pub(super) fn invalid_input(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn failure(class: &str, error: &dyn std::fmt::Display) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

struct DecodedUpload {
    file_name: String,
    file_content: Vec<u8>,
    file_type: Option<String>,
    has_type: bool,
}

fn tuple_text<'entry>(
    entry: &'entry [Json],
    index: usize,
) -> Result<&'entry str, Box<HelperError>> {
    match entry.get(index) {
        Some(Json::Text(text)) => Ok(text),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(invalid_input(format!(
                "a files tuple needs text at position {index}"
            )))
        }
    }
}

fn decoded_upload(entry: &[Json]) -> Result<DecodedUpload, Box<HelperError>> {
    let file_name = tuple_text(entry, 0)?.to_owned();
    let encoded = tuple_text(entry, 1)?;
    let file_content = STANDARD
        .decode(encoded)
        .map_err(|error| invalid_input(error.to_string()))?;
    let (file_type, has_type) = match entry.get(2) {
        None => (None, false),
        Some(Json::Null) => (None, true),
        Some(Json::Text(text)) => (Some(text.clone()), true),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(invalid_input(
                "a files tuple carries a text content type or null".to_owned(),
            ));
        }
    };
    Ok(DecodedUpload {
        file_name,
        file_content,
        file_type,
        has_type,
    })
}

pub(super) fn upload_of(upload: &DecodedUpload) -> SkillUpload<'_> {
    SkillUpload {
        file_name: &upload.file_name,
        content: &upload.file_content,
        file_type: if upload.has_type {
            SkillUploadType::Given(upload.file_type.as_deref())
        } else {
            SkillUploadType::Defaulted
        },
    }
}

pub(super) fn decoded_content(text: Option<&str>) -> Result<Option<Vec<u8>>, Box<HelperError>> {
    let Some(encoded) = text else {
        return Ok(None);
    };
    STANDARD
        .decode(encoded)
        .map(Some)
        .map_err(|error| invalid_input(error.to_string()))
}

fn decoded_files(step: &Step) -> Result<(Vec<DecodedUpload>, Option<Vec<u8>>), Box<HelperError>> {
    let entries = step.files.as_deref().unwrap_or(&[]);
    let mut decoded = Vec::with_capacity(entries.len());
    for entry in entries {
        decoded.push(decoded_upload(entry)?);
    }
    Ok((decoded, decoded_content(step.file_content.as_deref())?))
}

pub(super) fn creation_of<'request>(
    step: &'request Step,
    caller: Option<&'request Principal>,
    uploads: &'request [SkillUpload<'request>],
    content: Option<&'request [u8]>,
) -> HostedSkillCreation<'request> {
    HostedSkillCreation {
        display_title: step.display_title.as_deref(),
        description: step.description.as_deref(),
        instructions: step.instructions.as_deref(),
        files: step.files.is_some().then_some(uploads),
        file_content: content,
        file_name: step.file_name.as_deref(),
        file_type: step.file_type.as_deref(),
        metadata: step.metadata.as_ref(),
        user_id: step.user_id.as_deref(),
        principal: caller,
        litellm_call_id: step.litellm_call_id.as_deref(),
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let scripted = Scripted::new(input)?;
        let uuid = input.uuid.clone();
        let ids: SkillIdSource = Arc::new(move || uuid.clone());
        let clock: Clock = Arc::new(|| CACHE_CLOCK);
        let store = HostedSkillStore::new(scripted.clone(), ids, clock)
            .map_err(|error| failure("StoreError", &error))?;
        let ops = HostedSkillOps { store };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| invalid_input(error.to_string()))?;
        let answers = runtime.block_on(run_steps(&ops, &input.steps))?;
        Ok(scripted.response(&answers))
    }
}
