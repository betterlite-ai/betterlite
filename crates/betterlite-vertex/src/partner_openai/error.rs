use betterlite_openai::chat_choices::choices::ChoicesError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PartnerOpenAiError {
    #[error("the partner reply does not have the shape of a chat completion")]
    Reply {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Choices(#[from] ChoicesError),
}
