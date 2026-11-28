use betterlite_config::sections::jwt_settings::records::{IssuerConfig, Selector};
use betterlite_models::decisions::Json;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) scenarios: Vec<Scenario>,
}

// One verifier per scenario: the fakes, the env scoping and the identity provider's key stay together.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Scenario {
    pub(super) audience: Option<String>,
    pub(super) issuer: Option<String>,
    pub(super) userinfo_endpoint: Option<String>,
    pub(super) userinfo_ttl: Option<f64>,
    #[serde(default)]
    pub(super) issuers: Vec<IssuerEntry>,
    pub(super) now: f64,
    pub(super) key: Json,
    pub(super) steps: Vec<Step>,
}

// The LiteLLM_JWTAuth.issuers entry; the fields this operation never reads stay at their defaults.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct IssuerEntry {
    pub(super) issuer: String,
    pub(super) jwks_url: Option<String>,
    pub(super) audience: Option<Selector>,
    #[serde(default)]
    pub(super) disable_audience_validation: bool,
    pub(super) user_id_jwt_field: Option<String>,
    pub(super) user_email_jwt_field: Option<String>,
    pub(super) team_id_jwt_field: Option<String>,
    pub(super) team_ids_jwt_field: Option<String>,
    pub(super) org_id_jwt_field: Option<String>,
    pub(super) end_user_id_jwt_field: Option<String>,
}

impl IssuerEntry {
    pub(super) fn config(&self) -> IssuerConfig {
        IssuerConfig {
            issuer: self.issuer.clone(),
            jwks_url: self.jwks_url.clone(),
            audience: self.audience.clone(),
            disable_audience_validation: self.disable_audience_validation,
            user_id_jwt_field: self.user_id_jwt_field.clone(),
            user_email_jwt_field: self.user_email_jwt_field.clone(),
            team_id_jwt_field: self.team_id_jwt_field.clone(),
            team_ids_jwt_field: self.team_ids_jwt_field.clone(),
            org_id_jwt_field: self.org_id_jwt_field.clone(),
            end_user_id_jwt_field: self.end_user_id_jwt_field.clone(),
            virtual_key_claim_field: None,
            unregistered_client: None,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Step {
    pub(super) op: Op,
    pub(super) token: Option<String>,
    #[serde(default)]
    pub(super) http: Vec<Reply>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Op {
    AuthJwt,
    Decode,
    Userinfo,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Reply {
    pub(super) kind: ReplyKind,
    #[serde(default)]
    pub(super) status: u16,
    #[serde(default)]
    pub(super) body: String,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ReplyKind {
    Answer,
    Transport,
}
