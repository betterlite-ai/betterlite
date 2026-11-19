use std::collections::BTreeMap;

use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType};
use betterlite_config::sections::Section;
use betterlite_config::sections::file_upload_options::Raw;
use betterlite_core::api::file_upload_options::error::ResourceKind;
use betterlite_core::api::file_upload_options::managed::{
    validate_managed_id, validate_managed_upload,
};
use betterlite_core::api::file_upload_options::{
    FileCreationRequest, RequestBodyRead, extract_file_creation_params,
};
use betterlite_core::api::request_body::JsonObject;
use betterlite_core::api::request_form::values::{FormPart, FormParts, PartValue, UploadFile};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Call {
    Extract(ExtractCall),
    ManagedUpload(ManagedUploadCall),
    ManagedId(ManagedIdCall),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExtractCall {
    request_body: Option<JsonObject>,
    body: Option<JsonObject>,
    query_params: Option<BTreeMap<String, String>>,
    headers: Option<BTreeMap<String, String>>,
    form_parts: Option<Vec<FormPartInput>>,
    target_model_names_form: Option<String>,
    target_storage_form: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum FormPartInput {
    Text { name: String, text: String },
    Upload { name: String, filename: String },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManagedUploadCall {
    target_model_names: Vec<String>,
    model: Option<String>,
    require_managed_files: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManagedIdCall {
    resource_id: Option<String>,
    resource_kind: KindName,
    require_managed_files: Value,
}

#[derive(Debug, Clone, Copy, Deserialize)]
enum KindName {
    #[serde(rename = "file")]
    File,
    #[serde(rename = "batch")]
    Batch,
    #[serde(rename = "fine-tuning job")]
    FineTuningJob,
}

impl KindName {
    const fn kind(self) -> ResourceKind {
        match self {
            Self::File => ResourceKind::File,
            Self::Batch => ResourceKind::Batch,
            Self::FineTuningJob => ResourceKind::FineTuningJob,
        }
    }
}

struct ScriptedBody {
    body: Option<JsonObject>,
}

impl RequestBodyRead for ScriptedBody {
    fn request_body(&self) -> Option<JsonObject> {
        self.body.clone()
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let answers = input
            .calls
            .iter()
            .map(run_call)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Value::Array(answers))
    }
}

fn run_call(call: &Call) -> Result<Value, Box<HelperError>> {
    match call {
        Call::Extract(payload) => extract_answer(payload),
        Call::ManagedUpload(payload) => Ok(managed_upload_answer(payload)),
        Call::ManagedId(payload) => managed_id_answer(payload),
    }
}

fn extract_answer(call: &ExtractCall) -> Result<Value, Box<HelperError>> {
    let reader = ScriptedBody {
        body: call.body.clone(),
    };
    let parts = form_parts(call.form_parts.as_deref().unwrap_or_default());
    let query_params = call.query_params.clone().unwrap_or_default();
    let headers = header_map(call.headers.as_ref());
    let request = FileCreationRequest {
        query_params: &query_params,
        headers: &headers,
        form_parts: &parts,
        read_body: &reader,
    };
    let names_form = call
        .target_model_names_form
        .as_ref()
        .map(|name| Json::Text(name.clone()));
    let params = extract_file_creation_params(
        &request,
        call.request_body.as_ref(),
        names_form.as_ref(),
        call.target_storage_form.as_deref(),
    )
    .or_value_error()?;
    Ok(json!({
        "target_storage": params.target_storage,
        "target_model_names": params.target_model_names,
        "model": params.model,
    }))
}

fn managed_upload_answer(call: &ManagedUploadCall) -> Value {
    let enabled = gate(&call.require_managed_files);
    match validate_managed_upload(enabled, &call.target_model_names, call.model.as_deref()) {
        Ok(()) => json!({"ok": true}),
        Err(error) => json!({"status": error.status(), "detail": error.to_string()}),
    }
}

fn managed_id_answer(call: &ManagedIdCall) -> Result<Value, Box<HelperError>> {
    let enabled = gate(&call.require_managed_files);
    let runtime = Builder::new_current_thread().build()?;
    let outcome = runtime.block_on(validate_managed_id(
        enabled,
        call.resource_id.as_deref(),
        call.resource_kind.kind(),
        None,
        &principal(),
    ));
    Ok(match outcome {
        Ok(()) => json!({"ok": true}),
        Err(error) => json!({"status": error.status(), "detail": error.to_string()}),
    })
}

fn gate(value: &Value) -> bool {
    let source = json!({
        "litellm_settings": {"require_managed_files": value},
    });
    let Ok(raw) = serde_json::from_value::<Raw>(source) else {
        return false;
    };
    let lookup = BTreeMap::<String, String>::new();
    Section::convert(raw, &lookup).is_ok_and(|settings| settings.require_managed_files)
}

fn header_map(headers: Option<&BTreeMap<String, String>>) -> BTreeMap<String, String> {
    headers
        .into_iter()
        .flatten()
        .map(|(name, value)| (name.to_lowercase(), value.clone()))
        .collect()
}

fn form_parts(parts: &[FormPartInput]) -> FormParts {
    FormParts {
        items: parts.iter().map(form_part).collect(),
    }
}

fn form_part(part: &FormPartInput) -> FormPart {
    match part {
        FormPartInput::Text { name, text } => FormPart {
            name: name.clone(),
            value: PartValue::Text(text.clone()),
        },
        FormPartInput::Upload { name, filename } => FormPart {
            name: name.clone(),
            value: PartValue::Upload(UploadFile {
                filename: filename.clone(),
                content_type: None,
                content: Vec::new(),
                size: None,
            }),
        },
    }
}

fn principal() -> Principal {
    Principal {
        principal_type: PrincipalType::Human,
        subject: "subject".to_owned(),
        issuer: None,
        audience: Vec::new(),
        user: None,
        organization: None,
        teams: Vec::new(),
        project: None,
        end_user: None,
        roles: Vec::new(),
        scopes: Vec::new(),
        auth_method: AuthMethod::ApiKey,
        credential_ref: CredentialRef::default(),
        network: NetworkContext::default(),
        source_key: SourceKey::default(),
    }
}
