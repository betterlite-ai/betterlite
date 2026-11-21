use std::collections::BTreeMap;

use betterlite_config::sections::request_headers::UserHeaderMapping;
use betterlite_core::api::request_destination::RequestData;
use betterlite_core::api::request_principal_metadata::PrincipalFacts;
use serde::Deserialize;

pub(crate) type Pairs = Vec<(String, String)>;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) clean: Vec<CleanCall>,
    pub(super) backend: Vec<BackendCall>,
    pub(super) model_group: Vec<ModelGroupCall>,
    pub(super) metadata: Vec<MetadataCall>,
    pub(super) otel_trace_id: Vec<OtelCall>,
    pub(super) traceparent: Vec<TraceparentCall>,
    pub(super) user: Vec<UserCall>,
    pub(super) provider: Vec<Pairs>,
    pub(super) cache_control: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CleanCall {
    pub(super) headers: Pairs,
    pub(super) litellm_key_header_name: Option<String>,
    pub(super) forward_llm_provider_auth_headers: bool,
    pub(super) authenticated_with_header: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GeneralFlags {
    pub(super) forward_client_headers_to_llm_api: bool,
    pub(super) forward_openai_org_id: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BackendCall {
    pub(super) headers: Pairs,
    pub(super) request_data: RequestData,
    pub(super) principal: PrincipalFacts,
    pub(super) general_settings: GeneralFlags,
    pub(super) add_user_information: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ModelGroupCall {
    pub(super) data: RequestData,
    pub(super) headers: Pairs,
    pub(super) principal: PrincipalFacts,
    pub(super) team_model_aliases: BTreeMap<String, String>,
    pub(super) models: Option<Vec<String>>,
    pub(super) add_user_information: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MetadataCall {
    pub(super) headers: Pairs,
    pub(super) data: RequestData,
    pub(super) slot: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OtelCall {
    pub(super) data: RequestData,
    pub(super) slot: String,
    // the parent span's trace id and span id
    pub(super) span: Option<(u64, u64)>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TraceparentCall {
    pub(super) data: RequestData,
    pub(super) headers: Pairs,
    pub(super) forward: bool,
    pub(super) otel_enabled: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UserGeneralSettings {
    pub(super) user_header_name: Option<String>,
    pub(super) user_header_mappings: Option<Vec<UserHeaderMapping>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UserCall {
    pub(super) headers: Pairs,
    pub(super) general_settings: UserGeneralSettings,
    pub(super) principal_user_id: Option<String>,
}
