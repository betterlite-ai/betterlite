use std::collections::BTreeMap;

use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_config::sections::responses_id_security::Settings;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_secrets::secret_codec::diagnostics::SecretDiagnostics;
use betterlite_secrets::secret_codec::{NonceSource, StoredCipher, StoredValue};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use crate::api::api_boundaries::ApiLimits;

pub mod error;
pub mod owned_id;
mod rewrite;
mod tests;

use self::error::{Error, Forbidden};
use self::owned_id::{DecodedResponseId, OwnedIdCipher};

pub const ADDRESSED_RESPONSE_ID_FIELD: &str = "_litellm_addressed_response_id";
const RESPONSE_ID_FIELD: &str = "response_id";
const PREVIOUS_RESPONSE_ID_FIELD: &str = "previous_response_id";
const ADVERTISED_ID_PREFIX: &str = "resp_";
const MANAGED_ID_PREFIX: &str = "litellm_proxy";
const NO_SIGNING_KEY_MESSAGE: &str = "Response ID encryption is enabled but no signing key is configured. Please set LITELLM_SALT_KEY environment variable or configure a master_key. Skipping response ID encryption. See: https://docs.litellm.ai/docs/proxy/prod#5-set-litellm-salt-key";

// litellm/proxy/hooks/__init__.py lists this hook in PROXY_HOOKS, so startup installs it with no callbacks opt-in
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProxyHook {
    pub name: &'static str,
    pub opt_in: bool,
}

pub const PROXY_HOOKS: [ProxyHook; 1] = [ProxyHook {
    name: "responses_id_security",
    opt_in: false,
}];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponsesCall {
    Create,
    Get,
    Delete,
    Cancel,
    ListInputItems,
}

impl ResponsesCall {
    #[must_use]
    pub fn from_call_type(call_type: &str) -> Option<Self> {
        match call_type {
            "aresponses" => Some(Self::Create),
            "aget_responses" => Some(Self::Get),
            "adelete_responses" => Some(Self::Delete),
            "acancel_responses" => Some(Self::Cancel),
            "alist_input_items" => Some(Self::ListInputItems),
            _ => None,
        }
    }

    const fn addressed_field(self) -> &'static str {
        match self {
            Self::Create => PREVIOUS_RESPONSE_ID_FIELD,
            Self::Get | Self::Delete | Self::Cancel | Self::ListInputItems => RESPONSE_ID_FIELD,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallOutcome {
    NotResponsesCall,
    Unchanged,
    Authorized {
        routing_id: String,
        addressed_id: String,
    },
}

pub struct ResponsesIdSecurity<'policy, N, D> {
    pub settings: &'policy Settings,
    pub cipher: &'policy StoredCipher<N, D>,
    pub limits: ApiLimits,
}

impl<N, D> ResponsesIdSecurity<'_, N, D>
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    pub fn authorize_call(
        &self,
        call: Option<ResponsesCall>,
        data: &mut IndexMap<String, Json>,
        principal: &Principal,
    ) -> Result<CallOutcome, Error> {
        let Some(call) = call else {
            return Ok(CallOutcome::NotResponsesCall);
        };
        let field = call.addressed_field();
        let retained = data
            .get(ADDRESSED_RESPONSE_ID_FIELD)
            .and_then(as_nonempty_text);
        let Some(addressed) = retained.or_else(|| data.get(field).and_then(as_nonempty_text))
        else {
            return Ok(CallOutcome::Unchanged);
        };
        let addressed_id = addressed.to_owned();
        let routing_id = self.authorize_id(&addressed_id, principal)?;
        data.insert(field.to_owned(), Json::Text(routing_id.clone()));
        data.insert(
            ADDRESSED_RESPONSE_ID_FIELD.to_owned(),
            Json::Text(addressed_id.clone()),
        );
        Ok(CallOutcome::Authorized {
            routing_id,
            addressed_id,
        })
    }

    fn authorize_id(&self, response_id: &str, principal: &Principal) -> Result<String, Error> {
        let decoded =
            owned_id::decode_owned_response_id_text(response_id, self.cipher, self.limits)?;
        match decoded {
            DecodedResponseId::Managed {
                original_id,
                user_id,
                team_id,
            } => {
                self.check_access(&user_id, &team_id, principal)?;
                Ok(original_id)
            }
            DecodedResponseId::Unmanaged => {
                if self.unmanaged_allowed(principal) {
                    Ok(response_id.to_owned())
                } else {
                    Err(Error::Forbidden(Forbidden::Unmanaged))
                }
            }
        }
    }

    #[must_use]
    pub fn unmanaged_allowed(&self, principal: &Principal) -> bool {
        self.settings.disable_responses_id_security
            || self.settings.allow_unmanaged_response_ids
            || self.settings.signing_key.is_none()
            || principal.roles.contains(&Role::PlatformAdmin)
    }

    // LiteLLM checks the user before the team, and disabling security only logs the mismatch
    pub fn check_access(
        &self,
        user_id: &str,
        team_id: &str,
        principal: &Principal,
    ) -> Result<(), Error> {
        if principal.roles.contains(&Role::PlatformAdmin) {
            return Ok(());
        }
        let caller_user = principal.user.as_ref().map(|user| user.id.as_str());
        if !user_id.is_empty() && caller_user != Some(user_id) {
            if self.settings.disable_responses_id_security {
                tracing::debug!(
                    "Responses ID Security is disabled. User {} is accessing response id {} which is not associated with them.",
                    caller_user.map_or("None", |value| value),
                    user_id
                );
                return Ok(());
            }
            return Err(Error::Forbidden(Forbidden::User));
        }
        let caller_team = principal.teams.first().map(|team| team.id.as_str());
        if !team_id.is_empty() && caller_team != Some(team_id) {
            if self.settings.disable_responses_id_security {
                tracing::debug!(
                    "Responses ID Security is disabled. Response belongs to team {} but user {} is accessing it with team id {}.",
                    team_id,
                    caller_user.map_or("None", |value| value),
                    caller_team.map_or("None", |value| value)
                );
                return Ok(());
            }
            return Err(Error::Forbidden(Forbidden::Team));
        }
        Ok(())
    }

    // the nonstream success hook encrypts any ResponsesAPIResponse and keeps no request cache
    pub fn encrypt_response(
        &self,
        response: &mut ResponsesApiResponse,
        principal: &Principal,
    ) -> Result<(), Error> {
        if self.settings.disable_responses_id_security {
            return Ok(());
        }
        let Some(_key) = self.settings.signing_key.as_ref() else {
            tracing::debug!("{}", NO_SIGNING_KEY_MESSAGE);
            return Ok(());
        };
        let mut cache = BTreeMap::new();
        let mut rewrite = |original: &str| self.encrypted_id(original, principal, &mut cache);
        rewrite::rewrite_response(response, &mut rewrite)
    }

    // one cache per stream, so every event in a request reuses that request's ciphertext
    pub fn rewrite_stream_chunk(
        &self,
        request_route: Option<&str>,
        event: &mut ResponsesAPIStreamingResponse,
        principal: &Principal,
        cache: &mut BTreeMap<String, String>,
    ) -> Result<(), Error> {
        if self.settings.disable_responses_id_security || !rewrite::is_create_route(request_route) {
            return Ok(());
        }
        let Some(_key) = self.settings.signing_key.as_ref() else {
            tracing::debug!("{}", NO_SIGNING_KEY_MESSAGE);
            return Ok(());
        };
        let mut rewrite = |original: &str| self.encrypted_id(original, principal, &mut *cache);
        rewrite::rewrite_event(event, &mut rewrite)
    }

    fn encrypted_id(
        &self,
        original_id: &str,
        principal: &Principal,
        cache: &mut BTreeMap<String, String>,
    ) -> Result<String, Error> {
        if let Some(cached) = cache.get(original_id) {
            return Ok(cached.clone());
        }
        let user_id = principal.user.as_ref().map_or("", |user| user.id.as_str());
        let team_id = principal.teams.first().map_or("", |team| team.id.as_str());
        let payload = format!(
            "{MANAGED_ID_PREFIX}:responses_api:response_id:{original_id};user_id:{user_id};team_id:{team_id}"
        );
        let sealed: StoredValue<()> = self
            .cipher
            .encrypt_value(StoredValue::Text(SecretString::from(payload)), None)
            .map_err(|source| Error::Encrypt { source })?;
        let StoredValue::Text(sealed) = sealed else {
            return Err(Error::NonTextCiphertext);
        };
        let advertised = format!("{ADVERTISED_ID_PREFIX}{}", sealed.expose_secret());
        cache.insert(original_id.to_owned(), advertised.clone());
        Ok(advertised)
    }
}

// LiteLLM reads the field with isinstance(str) and a truthiness check, so only a nonempty string counts
fn as_nonempty_text(value: &Json) -> Option<&str> {
    match value {
        Json::Text(text) if !text.is_empty() => Some(text),
        Json::Text(_)
        | Json::Null
        | Json::Bool(_)
        | Json::Number(_)
        | Json::List(_)
        | Json::Object(_) => None,
    }
}
