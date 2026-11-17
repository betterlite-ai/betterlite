use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;

use super::decode::{TypedValue, decode_group, to_json};
use super::error::DecodeError;
use super::regenerate::RegenerateKeyRequest;
use super::request::GenerateKeyRequest;
use super::update::UpdateKeyRequest;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyOperation {
    Generate,
    Update,
    Regenerate,
}

#[derive(Clone)]
pub enum PolicyRequestBody {
    Generate(GenerateKeyRequest),
    Update(UpdateKeyRequest),
    Regenerate(RegenerateKeyRequest),
}

#[derive(Clone)]
pub struct CustomKeyPolicyRequest {
    pub operation: PolicyOperation,
    pub existing_key: Option<LiteLLMVerificationToken>,
    pub effective_key: LiteLLMVerificationToken,
    pub request: PolicyRequestBody,
}

// outer None is a missing key and inner None an explicit null, so the two stay apart
type NullableKey = Option<Option<LiteLLMVerificationToken>>;

// `existing_key` is nullable but has no default, so its presence must survive decoding.
#[derive(serde::Deserialize)]
struct PolicyEnvelope {
    operation: PolicyOperation,
    #[serde(default, with = "serde_with::rust::double_option")]
    existing_key: NullableKey,
    effective_key: LiteLLMVerificationToken,
    request: JsonData,
}

impl CustomKeyPolicyRequest {
    pub fn read(value: &TypedValue) -> Result<Self, DecodeError> {
        let envelope: PolicyEnvelope = decode_group(&value.object()?)?;
        let Some(existing_key) = envelope.existing_key else {
            return Err(DecodeError::MissingField {
                field: "existing_key",
            });
        };
        Ok(Self {
            operation: envelope.operation,
            existing_key,
            effective_key: envelope.effective_key,
            request: select_request(&TypedValue {
                json: envelope.request,
            })?,
        })
    }

    pub fn project(&self) -> Result<BTreeMap<String, JsonData>, DecodeError> {
        let (kind, request) = match &self.request {
            PolicyRequestBody::Generate(request) => {
                ("GenerateKeyRequest", request.project()?.resolved)
            }
            PolicyRequestBody::Update(request) => ("UpdateKeyRequest", request.project()?.resolved),
            PolicyRequestBody::Regenerate(request) => {
                ("RegenerateKeyRequest", request.project()?.resolved)
            }
        };
        Ok(BTreeMap::from([
            ("operation".to_owned(), to_json(&self.operation)?),
            ("request_kind".to_owned(), JsonData::String(kind.to_owned())),
            ("request".to_owned(), JsonData::Object(request)),
        ]))
    }
}

// Pydantic's smart union keeps the member with the most matching fields, ties to declaration order.
fn select_request(value: &TypedValue) -> Result<PolicyRequestBody, DecodeError> {
    let mut selected: Option<(usize, PolicyRequestBody)> =
        if let Ok(request) = GenerateKeyRequest::read(value) {
            Some((
                request.project()?.supplied.len(),
                PolicyRequestBody::Generate(request),
            ))
        } else {
            None
        };
    if let Ok(request) = UpdateKeyRequest::read(value) {
        let score = request.project()?.supplied.len();
        if selected.as_ref().is_none_or(|(best, _)| score > *best) {
            selected = Some((score, PolicyRequestBody::Update(request)));
        }
    }
    if let Ok(request) = RegenerateKeyRequest::read(value) {
        let score = request.project()?.supplied.len();
        if selected.as_ref().is_none_or(|(best, _)| score > *best) {
            selected = Some((score, PolicyRequestBody::Regenerate(request)));
        }
    }
    match selected {
        Some((_, request)) => Ok(request),
        None => Err(DecodeError::RequestUnion),
    }
}
