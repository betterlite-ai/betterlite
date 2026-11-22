use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use secrecy::SecretString;

use crate::audit_events::identity::CallerIdentity;
use crate::key_fields::regenerate::RegenerateKeyRequest;
use crate::key_fields::request::GenerateKeyRequest;
use crate::key_fields::response::GenerateKeyResponse;
use crate::key_fields::update::UpdateKeyRequest;

pub struct GeneratedEvent {
    pub data: GenerateKeyRequest,
    pub response: GenerateKeyResponse,
}

pub struct UpdatedEvent {
    pub data: UpdateKeyRequest,
    pub row: LiteLLMVerificationToken,
    // the route's UpdateKeyResponse; the hook never reads it
    pub response: JsonData,
}

pub struct RotatedEvent {
    // /key/{key}/regenerate sends no request body
    pub data: Option<RegenerateKeyRequest>,
    pub row: LiteLLMVerificationToken,
    pub response: GenerateKeyResponse,
}

pub struct DeletedEvent {
    // KeyRequest and KeyDeletedResponse carry nothing the hook reads; the rows hold what it needs
    pub request: JsonData,
    pub rows: Vec<JsonData>,
    pub response: JsonData,
}

pub enum KeyEvent {
    Generated(GeneratedEvent),
    Updated(UpdatedEvent),
    Rotated(RotatedEvent),
    Deleted(DeletedEvent),
}

// UserAPIKeyAuth: api_key names the caller of created/updated rows, token names the caller of
// rotated/deleted rows, and the metadata decides whether the changed-by header may stand in
#[derive(Clone, Default)]
pub struct KeyEventCaller {
    pub identity: CallerIdentity,
    pub token: Option<SecretString>,
}
