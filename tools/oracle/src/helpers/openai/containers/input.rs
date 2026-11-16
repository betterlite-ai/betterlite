use std::collections::BTreeMap;

use betterlite_openai::api::container_contracts::config::ContainerListQuery;
use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    pub(super) supported: bool,
    #[serde(default)]
    pub(super) maps: Vec<MapCall>,
    #[serde(default)]
    pub(super) joins: Vec<JoinCall>,
    #[serde(default)]
    pub(super) urls: Vec<UrlCall>,
    #[serde(default)]
    pub(super) creates: Vec<CreateCall>,
    #[serde(default)]
    pub(super) requests: Vec<RequestCall>,
    #[serde(default)]
    pub(super) headers: Vec<HeadersCall>,
    #[serde(default)]
    pub(super) replies: Vec<ReplyCall>,
    #[serde(default)]
    pub(super) content_replies: Vec<BodyCall>,
    #[serde(default)]
    pub(super) errors: Vec<ErrorCall>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Globals {
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<String>,
    pub(super) openai_key: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MapCall {
    #[serde(default)]
    pub(super) params: Map<String, Value>,
    pub(super) drop_params: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JoinCall {
    pub(super) api_base: String,
    pub(super) path_suffix: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlCall {
    pub(super) api_base: Option<String>,
    pub(super) globals: Globals,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateCall {
    pub(super) name: String,
    #[serde(default)]
    pub(super) params: Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct QueryCall {
    after: Option<String>,
    limit: Option<i64>,
    order: Option<String>,
    extra_query: Option<Map<String, Value>>,
}

impl<'call> From<&'call QueryCall> for ContainerListQuery<'call> {
    fn from(query: &'call QueryCall) -> Self {
        Self {
            after: query.after.as_deref(),
            limit: query.limit,
            order: query.order.as_deref(),
            extra_query: query.extra_query.as_ref(),
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum RequestCall {
    List {
        api_base: String,
        query: QueryCall,
    },
    FileList {
        container_id: String,
        api_base: String,
        query: QueryCall,
    },
    Retrieve {
        container_id: String,
        api_base: String,
    },
    Delete {
        container_id: String,
        api_base: String,
    },
    Content {
        container_id: String,
        file_id: String,
        api_base: String,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeadersCall {
    pub(super) api_key: Option<String>,
    #[serde(default)]
    pub(super) headers: BTreeMap<String, String>,
    pub(super) globals: Globals,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReplyKind {
    Create,
    Retrieve,
    List,
    Delete,
    FileList,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyCall {
    pub(super) kind: ReplyKind,
    pub(super) body: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BodyCall {
    pub(super) body: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorCall {
    pub(super) message: String,
    pub(super) status: u16,
    #[serde(default)]
    pub(super) headers: BTreeMap<String, String>,
}
