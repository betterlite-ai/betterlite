pub mod from_auth;
pub mod parse;

use crate::otel::model::utils::coercion::OtelModelValue;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct RequestIdentity {
    pub call_id: Option<String>,
    pub team_id: Option<String>,
    pub team_alias: Option<String>,
    pub team_metadata: Option<BTreeMap<String, OtelModelValue>>,
    pub key_hash: Option<String>,
    pub end_user: Option<String>,
    pub provider_model: Option<String>,
    pub request_route: Option<String>,
    pub metadata: BTreeMap<String, String>,
}
