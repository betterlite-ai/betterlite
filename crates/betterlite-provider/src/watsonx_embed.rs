pub mod error;
pub mod reply;
pub mod request;

mod tests;

use betterlite_openai::embedding::{
    EmbeddingInput, EmbeddingRecord, EmbeddingResponse, EmbeddingUsage,
};
use betterlite_openai::watsonx_auth::credentials::get_api_params;
use betterlite_openai::watsonx_auth::environment::WatsonxEnvironment;
use betterlite_openai::watsonx_auth::payload::prepare_payload;
use betterlite_openai::watsonx_auth::types::{WatsonxEndpoint, WatsonxParams};
use betterlite_openai::watsonx_auth::urls::{add_api_version_to_url, get_base_url};

use self::error::WatsonxEmbeddingError;
use self::reply::WatsonxEmbeddingReply;
use self::request::WatsonxEmbeddingBody;

pub struct WatsonxEmbeddingConfig<'env> {
    pub environment: &'env WatsonxEnvironment,
}

impl WatsonxEmbeddingConfig<'_> {
    pub fn get_complete_url(
        &self,
        api_base: Option<&str>,
        options: &mut WatsonxParams,
    ) -> Result<String, WatsonxEmbeddingError> {
        let base = get_base_url(api_base, self.environment)?;
        let url = format!(
            "{}{}",
            base.trim_end_matches('/'),
            WatsonxEndpoint::Embeddings.path()
        );
        let version = options
            .remove("api_version")
            .map(serde_json::from_value::<Option<String>>)
            .transpose()
            .map_err(|source| WatsonxEmbeddingError::InvalidField {
                field: "api_version",
                source,
            })?
            .flatten();
        Ok(add_api_version_to_url(&url, version.as_deref()))
    }

    pub fn transform_embedding_request(
        &self,
        model: &str,
        input: &EmbeddingInput,
        mut options: WatsonxParams,
    ) -> Result<WatsonxEmbeddingBody, WatsonxEmbeddingError> {
        let api_params = get_api_params(&mut options, Some(model), self.environment)?;
        let identity = prepare_payload(model, &api_params);
        let inputs = request::inputs(input)?;
        Ok(WatsonxEmbeddingBody {
            inputs,
            parameters: options,
            identity,
        })
    }

    #[must_use]
    pub fn transform_embedding_response(
        model: &str,
        reply: WatsonxEmbeddingReply,
    ) -> EmbeddingResponse {
        let prompt_tokens = reply.input_token_count.unwrap_or_default();
        let data = (0_u32..)
            .zip(reply.results)
            .map(|(index, result)| EmbeddingRecord {
                object: String::from("embedding"),
                embedding: result.embedding,
                index,
            })
            .collect();
        EmbeddingResponse {
            object: String::from("list"),
            data,
            model: model.to_owned(),
            usage: EmbeddingUsage {
                prompt_tokens,
                completion_tokens: 0,
                completion_tokens_details: None,
                prompt_tokens_details: None,
                total_tokens: prompt_tokens,
            },
        }
    }
}
