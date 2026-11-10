use async_trait::async_trait;
use secrecy::SecretString;

use crate::gigachat_chat::error::GigaChatError;
use crate::gigachat_utils::IdSource;

#[async_trait]
pub trait GigachatBearer: Send + Sync {
    async fn access_token(
        &self,
        credentials: Option<&SecretString>,
    ) -> Result<SecretString, GigaChatError>;
}

#[async_trait]
pub trait GigachatImageUploader: Send + Sync {
    async fn upload(&self, image_url: &str) -> Option<String>;
}

pub struct GigachatChatConfig<'deps> {
    pub bearer: &'deps dyn GigachatBearer,
    pub uploader: &'deps dyn GigachatImageUploader,
    pub ids: &'deps dyn IdSource,
}
