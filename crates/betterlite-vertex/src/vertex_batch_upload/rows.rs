use std::error::Error as StdError;

use betterlite_config::sections::vertex_schema_shaping::Settings as SchemaSettings;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::media::inline::RemoteMediaFetcher;

use crate::gemini_media::gcs_metadata::GcsDeps;
use crate::gemini_options::defaults::OptionSettings;
use crate::gemini_thinking::settings::{MinimalEffortSupport, ThinkingSettings};
use crate::model_policy::capabilities::ModelCapabilityLookup;
use crate::vertex_batch_upload::line::BatchLine;
use crate::vertex_batch_upload::rows::error::RowError;

pub mod chat;
pub mod embeddings;
pub mod error;

mod tests;

// the binder above this crate supplies both, because the Vertex crate must not import core
pub trait RowMappers {
    type Error: StdError + Send + 'static;

    fn responses_to_chat(&self, provider: &str, body: &Object) -> Result<Object, Self::Error>;

    fn developer_to_system(&self, messages: Vec<Object>) -> Result<Vec<Object>, Self::Error>;

    // the binder above holds the route's GCS client, and the row reads an extensionless gs:// object with it
    fn gcs_deps(&self) -> Option<GcsDeps<'_>> {
        None
    }
}

pub struct RowContext<'ctx, Mappers, Fetcher> {
    pub mappers: &'ctx Mappers,
    pub fetcher: &'ctx Fetcher,
    pub capabilities: &'ctx (dyn ModelCapabilityLookup + Sync),
    pub options: &'ctx OptionSettings,
    pub schema: &'ctx SchemaSettings,
    pub thinking: &'ctx ThinkingSettings,
    pub support: &'ctx (dyn MinimalEffortSupport + Sync),
    pub modify_params: bool,
}

// LiteLLM routes on the url without its query and trailing slashes, and a bare name or a longer path ending in it counts
pub async fn vertex_rows<Mappers, Fetcher>(
    line: &BatchLine,
    context: &RowContext<'_, Mappers, Fetcher>,
) -> Result<Vec<String>, RowError<Mappers::Error>>
where
    Mappers: RowMappers + Sync,
    Fetcher: RemoteMediaFetcher + Sync,
{
    let path = match &line.url {
        Some(Json::Text(url)) => url
            .split('?')
            .next()
            .unwrap_or_default()
            .trim_end_matches('/'),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => "",
    };
    if path == "embeddings" || path.ends_with("/embeddings") {
        return embeddings::rows(line);
    }
    let responses = path == "responses" || path.ends_with("/responses");
    chat::rows(line, responses, context).await
}
