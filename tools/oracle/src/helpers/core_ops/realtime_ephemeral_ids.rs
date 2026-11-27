use betterlite_core::api::realtime_ephemeral_ids::session::{
    RealtimeSessionType, model_candidates, set_transcription_model,
};
use betterlite_core::api::realtime_ephemeral_ids::token::{
    RealtimeTokenFields, TokenDecodeLimits, decode_realtime_token_payload,
    encode_realtime_token_payload,
};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;

const LIMITS: TokenDecodeLimits = TokenDecodeLimits { max_bytes: 1 << 20 };

#[derive(Debug, Deserialize)]
pub(crate) struct TokenFields {
    ephemeral_key: Json,
    model_id: String,
    user_id: Option<String>,
    team_id: Option<String>,
    expires_at: Json,
    session_type: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Function {
    SessionType {
        session_type: Option<String>,
    },
    Candidates {
        session: IndexMap<String, Json>,
    },
    SetModel {
        session: IndexMap<String, Json>,
        model: String,
        create_if_missing: bool,
    },
    Encode {
        token: TokenFields,
    },
    Decode {
        decrypted: String,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Function;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Function::SessionType { session_type } => {
                let resolved = RealtimeSessionType::from_request(session_type.as_deref());
                Ok(Value::String(resolved.as_str().to_owned()))
            }
            Function::Candidates { session } => {
                Ok(serde_json::to_value(model_candidates(session))?)
            }
            Function::SetModel {
                session,
                model,
                create_if_missing,
            } => {
                let mut updated = session.clone();
                set_transcription_model(&mut updated, model, *create_if_missing);
                Ok(serde_json::to_value(updated)?)
            }
            Function::Encode { token } => {
                let fields = RealtimeTokenFields {
                    ephemeral_key: &token.ephemeral_key,
                    model_id: &token.model_id,
                    user_id: token.user_id.as_deref(),
                    team_id: token.team_id.as_deref(),
                    expires_at: &token.expires_at,
                    session_type: &token.session_type,
                };
                Ok(Value::String(encode_realtime_token_payload(&fields)))
            }
            Function::Decode { decrypted } => {
                match decode_realtime_token_payload(decrypted.as_bytes(), LIMITS) {
                    Some(token) => Ok(serde_json::to_value(token)?),
                    None => Ok(Value::Null),
                }
            }
        }
    }
}
