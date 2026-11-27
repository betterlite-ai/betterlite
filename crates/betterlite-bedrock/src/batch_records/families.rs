use std::error::Error;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

// The optional parameters of a chat body, in the order the body carried them.
pub type Params = IndexMap<String, Json>;

pub type Failure = Box<dyn Error + Send + Sync>;

// The provider passes a batch record needs, bound at composition because their sources live above this leaf.
#[async_trait]
pub trait BatchFamilies: Send + Sync {
    // get_optional_params for the invoke route model: the mapped parameters the request handler keeps.
    async fn anthropic_params(
        &self,
        mapping_model: &str,
        params: &Params,
    ) -> Result<Params, Failure>;

    // AmazonAnthropicClaudeConfig.transform_request over the projected messages.
    async fn anthropic_body(
        &self,
        model: &str,
        messages: &[Json],
        params: &Params,
    ) -> Result<Json, Failure>;

    // AmazonConverseConfig.map_openai_params then transform_request, in one pass.
    async fn nova_body(
        &self,
        model: &str,
        messages: &[Json],
        params: &Params,
    ) -> Result<Json, Failure>;

    // responses_batch_body_to_chat_body, the Responses-to-Chat bridge the real-time path uses.
    async fn responses_to_chat(&self, body: &Json) -> Result<Json, Failure>;

    // FilesAPIUtils.is_batch_jsonl_file.
    fn is_batch_jsonl(
        &self,
        purpose: Option<&str>,
        content_type: Option<&str>,
        body: Option<&[u8]>,
    ) -> bool;
}
