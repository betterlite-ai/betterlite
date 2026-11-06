use betterlite_openai::assistants::records::{AssistantCreate, ListParams};
use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OptionFields {
    pub(super) api_key: Option<String>,
    pub(super) api_base: Option<String>,
    pub(super) timeout: Option<f64>,
    pub(super) max_retries: Option<u32>,
    pub(super) organization: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply {
    pub(super) status: u16,
    pub(super) body: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListFields {
    #[serde(default)]
    pub(super) options: OptionFields,
    #[serde(default)]
    pub(super) params: ListParams,
    pub(super) flag: Option<bool>,
    #[serde(default)]
    pub(super) direct: bool,
    pub(super) reply: Option<Reply>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateFields {
    #[serde(default)]
    pub(super) options: OptionFields,
    pub(super) data: AssistantCreate,
    pub(super) flag: Option<bool>,
    pub(super) reply: Option<Reply>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DeleteFields {
    #[serde(default)]
    pub(super) options: OptionFields,
    pub(super) assistant_id: String,
    pub(super) flag: Option<bool>,
    pub(super) reply: Option<Reply>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum KindName {
    Sync,
    Async,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Supplied {
    pub(super) kind: KindName,
    pub(super) organization: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClientFields {
    pub(super) kind: KindName,
    pub(super) options: OptionFields,
    pub(super) supplied: Option<Supplied>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AssistantsCall {
    ListAssistants(ListFields),
    CreateAssistant(Box<CreateFields>),
    DeleteAssistant(DeleteFields),
    ClientArguments(ClientFields),
}

impl AssistantsCall {
    // LiteLLM treats an absent flag as the sync call, so the boundary maps a missing or null flag to false.
    pub(super) fn async_request(&self) -> bool {
        match self {
            Self::ListAssistants(fields) => fields.flag.unwrap_or(false),
            Self::CreateAssistant(fields) => fields.flag.unwrap_or(false),
            Self::DeleteAssistant(fields) => fields.flag.unwrap_or(false),
            Self::ClientArguments(_) => false,
        }
    }
}
