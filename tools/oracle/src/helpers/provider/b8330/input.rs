use std::collections::BTreeMap;

use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use betterlite_provider::data_cloud_dispatch::select::EmbeddingConfig;
use serde::Deserialize;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Operation {
    Chat,
    Embedding,
    OciStream,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Provider {
    Databricks,
    Snowflake,
    Oci,
}

impl Provider {
    pub(super) const fn slug(self) -> &'static str {
        match self {
            Self::Databricks => "databricks",
            Self::Snowflake => "snowflake",
            Self::Oci => "oci",
        }
    }

    pub(super) const fn chat_tag(self) -> ChatConfigTag {
        match self {
            Self::Databricks => ChatConfigTag::Databricks,
            Self::Snowflake => ChatConfigTag::Snowflake,
            Self::Oci => ChatConfigTag::OciChat,
        }
    }

    pub(super) const fn embedding_config(self) -> EmbeddingConfig {
        match self {
            Self::Databricks => EmbeddingConfig::Databricks,
            Self::Snowflake => EmbeddingConfig::Snowflake,
            Self::Oci => EmbeddingConfig::Oci,
        }
    }

    // only names both sides prepare themselves; the HTTP client adds the rest on its own
    pub(super) const fn kept_headers(self) -> &'static [&'static str] {
        match self {
            Self::Databricks => &["authorization", "content-type", "x-oracle-extra"],
            Self::Snowflake => &[
                "authorization",
                "content-type",
                "accept",
                "x-snowflake-authorization-token-type",
                "anthropic-version",
            ],
            Self::Oci => &["content-type", "user-agent"],
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Answer {
    pub(super) status: u16,
    pub(super) body: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Case {
    pub(super) op: Operation,
    pub(super) provider: Provider,
    pub(super) model: String,
    #[serde(default)]
    pub(super) messages: Vec<Message>,
    #[serde(default)]
    pub(super) input: Option<Json>,
    #[serde(default)]
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) api_key: Option<String>,
    #[serde(default)]
    pub(super) headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(super) stream: Option<bool>,
    #[serde(default)]
    pub(super) optional_params: Option<Json>,
    #[serde(default)]
    pub(super) env: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) answer: Option<Answer>,
}

impl Case {
    pub(super) fn optional_params(&self) -> Result<Object, Box<HelperError>> {
        match &self.optional_params {
            None | Some(Json::Null) => Ok(Object::new()),
            Some(value) => Ok(serde_json::from_value::<Object>(serde_json::to_value(
                value,
            )?)?),
        }
    }
}
