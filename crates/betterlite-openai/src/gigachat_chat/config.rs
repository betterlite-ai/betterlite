use async_trait::async_trait;
use secrecy::SecretString;

use crate::gigachat_chat::error::GigaChatError;
use crate::gigachat_utils::IdSource;

// The token seam B8312 fills over the GigaChat token source; this crate never fetches or caches a token
#[async_trait]
pub trait GigachatBearer: Send + Sync {
    async fn access_token(
        &self,
        credentials: Option<&SecretString>,
    ) -> Result<SecretString, GigaChatError>;
}

// The image seam B8178 fills over GigaChat's file upload; a URL that fails to upload keeps no id
#[async_trait]
pub trait GigachatImageUploader: Send + Sync {
    async fn upload(&self, image_url: &str) -> Option<String>;
}

pub struct GigachatChatConfig<'deps> {
    pub bearer: &'deps dyn GigachatBearer,
    pub uploader: &'deps dyn GigachatImageUploader,
    pub ids: &'deps dyn IdSource,
}
