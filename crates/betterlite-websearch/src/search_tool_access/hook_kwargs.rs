use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(from = "Box<RawValue>")]
pub struct ParentOtelSpan(SpanValue);

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
enum SpanValue {
    Json(Box<RawValue>),
    #[serde(skip)]
    Tracing(tracing::Span),
}

impl From<Box<RawValue>> for ParentOtelSpan {
    fn from(raw: Box<RawValue>) -> Self {
        Self(SpanValue::Json(raw))
    }
}

impl From<tracing::Span> for ParentOtelSpan {
    fn from(span: tracing::Span) -> Self {
        Self(SpanValue::Tracing(span))
    }
}

impl ParentOtelSpan {
    #[must_use]
    pub const fn handle(&self) -> Option<&tracing::Span> {
        match &self.0 {
            SpanValue::Json(_) => None,
            SpanValue::Tracing(span) => Some(span),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SearchCaller {
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub parent_otel_span: Option<ParentOtelSpan>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct HookMetadata {
    #[serde(default)]
    pub user_api_key_auth: Option<SearchCaller>,
    #[serde(default)]
    pub session_id: Option<Box<RawValue>>,
    #[serde(default)]
    pub trace_id: Option<Box<RawValue>>,
    #[serde(default)]
    pub litellm_parent_otel_span: Option<ParentOtelSpan>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct HookParams {
    #[serde(default)]
    pub metadata: Option<HookMetadata>,
    #[serde(default)]
    pub litellm_metadata: Option<HookMetadata>,
    #[serde(default)]
    pub litellm_session_id: Option<Box<RawValue>>,
    #[serde(default)]
    pub litellm_trace_id: Option<Box<RawValue>>,
    #[serde(default)]
    pub litellm_call_id: Option<Box<RawValue>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SearchHookKwargs {
    #[serde(default)]
    pub metadata: Option<HookMetadata>,
    #[serde(default)]
    pub litellm_metadata: Option<HookMetadata>,
    #[serde(default)]
    pub litellm_params: Option<HookParams>,
    #[serde(default)]
    pub litellm_session_id: Option<Box<RawValue>>,
    #[serde(default)]
    pub litellm_trace_id: Option<Box<RawValue>>,
    #[serde(default)]
    pub litellm_call_id: Option<Box<RawValue>>,
}

impl SearchHookKwargs {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.metadata.is_none()
            && self.litellm_metadata.is_none()
            && self.litellm_params.is_none()
            && self.litellm_session_id.is_none()
            && self.litellm_trace_id.is_none()
            && self.litellm_call_id.is_none()
    }
}

#[must_use]
pub fn caller_from_kwargs(kwargs: Option<&SearchHookKwargs>) -> Option<&SearchCaller> {
    let hook_kwargs = kwargs?;
    let top_level = [
        hook_kwargs.metadata.as_ref(),
        hook_kwargs.litellm_metadata.as_ref(),
    ];
    if let Some(caller) = top_level
        .into_iter()
        .flatten()
        .find_map(|metadata| metadata.user_api_key_auth.as_ref())
    {
        return Some(caller);
    }
    let params = hook_kwargs.litellm_params.as_ref()?;
    [params.metadata.as_ref(), params.litellm_metadata.as_ref()]
        .into_iter()
        .flatten()
        .find_map(|metadata| metadata.user_api_key_auth.as_ref())
}
