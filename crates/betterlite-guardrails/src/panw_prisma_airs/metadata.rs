use serde::Deserialize;

// The slice of LiteLLM's request data dict that _prepare_metadata_from_request reads.
#[derive(Debug, Default, Deserialize)]
pub struct RequestData {
    pub user: Option<String>,
    pub model: Option<String>,
    pub litellm_trace_id: Option<String>,
    pub metadata: Option<UserMetadata>,
}

// Also the shape of requester_metadata, which carries the same keys one level down.
#[derive(Debug, Default, Deserialize)]
pub struct UserMetadata {
    pub profile_name: Option<String>,
    pub profile_id: Option<String>,
    pub user_ip: Option<String>,
    pub app_name: Option<String>,
    pub app_user: Option<String>,
    pub litellm_trace_id: Option<String>,
    pub trace_id: Option<String>,
    pub requester_metadata: Option<Box<Self>>,
}

impl UserMetadata {
    // Fields the request left unset come from requester_metadata; the proxy stores the x-litellm-trace-id header as trace_id, so it ranks below an explicit litellm_trace_id.
    fn or_requester(&self, requester: &Self) -> Self {
        Self {
            profile_name: self
                .profile_name
                .clone()
                .or_else(|| requester.profile_name.clone()),
            profile_id: self
                .profile_id
                .clone()
                .or_else(|| requester.profile_id.clone()),
            user_ip: self.user_ip.clone().or_else(|| requester.user_ip.clone()),
            app_name: self.app_name.clone().or_else(|| requester.app_name.clone()),
            app_user: self.app_user.clone().or_else(|| requester.app_user.clone()),
            litellm_trace_id: self
                .litellm_trace_id
                .clone()
                .or_else(|| self.trace_id.clone())
                .or_else(|| requester.litellm_trace_id.clone()),
            trace_id: None,
            requester_metadata: None,
        }
    }
}

// The metadata dict _call_panw_api reads; app_name here is the caller's bare name, not yet prefixed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanMetadata {
    pub user: String,
    pub model: String,
    pub profile_name: Option<String>,
    pub profile_id: Option<String>,
    pub user_ip: Option<String>,
    pub requester_ip_address: Option<String>,
    pub app_name: Option<String>,
    pub app_user: Option<String>,
    pub litellm_trace_id: Option<String>,
}

// Mirrors LiteLLM's fn _prepare_metadata_from_request.
#[must_use]
pub fn prepare_metadata_from_request(data: &RequestData) -> ScanMetadata {
    let fallback = UserMetadata::default();
    let user_metadata = data.metadata.as_ref().unwrap_or(&fallback);
    let requester = user_metadata
        .requester_metadata
        .as_deref()
        .unwrap_or(&fallback);
    let merged = user_metadata.or_requester(requester);
    ScanMetadata {
        user: data
            .user
            .clone()
            .unwrap_or_else(|| "litellm_user".to_owned()),
        model: data.model.clone().unwrap_or_else(|| "unknown".to_owned()),
        profile_name: merged.profile_name,
        profile_id: merged.profile_id,
        user_ip: merged.user_ip,
        requester_ip_address: None,
        app_name: merged.app_name,
        app_user: merged.app_user,
        litellm_trace_id: data.litellm_trace_id.clone().or(merged.litellm_trace_id),
    }
}
