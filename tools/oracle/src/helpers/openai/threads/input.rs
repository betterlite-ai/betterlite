use betterlite_openai::threads::records::ThreadMessageData;
use serde::Deserialize;
use std::collections::BTreeMap;

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
pub(crate) struct CreateThreadFields {
    #[serde(default)]
    pub(super) options: OptionFields,
    pub(super) messages: Option<Vec<ThreadMessageData>>,
    pub(super) metadata: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(super) acreate_thread: Option<bool>,
    pub(super) reply: Option<Reply>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GetThreadFields {
    #[serde(default)]
    pub(super) options: OptionFields,
    pub(super) thread_id: String,
    #[serde(default)]
    pub(super) aget_thread: Option<bool>,
    pub(super) reply: Option<Reply>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AddMessageFields {
    #[serde(default)]
    pub(super) options: OptionFields,
    pub(super) thread_id: String,
    pub(super) data: ThreadMessageData,
    #[serde(default)]
    pub(super) a_add_message: Option<bool>,
    pub(super) reply: Option<Reply>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GetMessagesFields {
    #[serde(default)]
    pub(super) options: OptionFields,
    pub(super) thread_id: String,
    #[serde(default)]
    pub(super) aget_messages: Option<bool>,
    pub(super) reply: Option<Reply>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DispatchFields {
    #[serde(default)]
    pub(super) options: OptionFields,
    #[serde(default)]
    pub(super) acreate_thread: Option<bool>,
    #[serde(default)]
    pub(super) aget_thread: Option<bool>,
    #[serde(default)]
    pub(super) a_add_message: Option<bool>,
    #[serde(default)]
    pub(super) aget_messages: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ThreadsCall {
    CreateThread(CreateThreadFields),
    GetThread(GetThreadFields),
    AddMessage(AddMessageFields),
    GetMessages(GetMessagesFields),
    Dispatch(DispatchFields),
}
