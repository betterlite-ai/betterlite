use thiserror::Error;

use crate::chat_choices::endpoint::ModelListError;

#[derive(Debug, Error)]
pub enum ProxyError {
    #[error("litellm_proxy needs a gateway URL: pass api_base or set LITELLM_PROXY_API_BASE")]
    MissingApiBase,
    #[error("extra_body must be an object so thinking can be nested under it")]
    ExtraBodyNotObject,
    #[error("the proxy model list request failed")]
    ModelList {
        #[source]
        source: ModelListError,
    },
}
