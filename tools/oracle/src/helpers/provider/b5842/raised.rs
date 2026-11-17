use betterlite_openai::watsonx_audio::error::WatsonxAudioError;
use betterlite_openai::watsonx_passthrough::WatsonxPassthroughError;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_provider::watsonx_embed::error::WatsonxEmbeddingError;
use betterlite_provider::watsonx_rerank::error::WatsonxRerankError;

#[derive(Default)]
pub(super) struct Raised {
    status: Option<u16>,
}

macro_rules! raised_from {
    ($($error:ty),+) => {
        $(
            impl From<$error> for Raised {
                fn from(error: $error) -> Self { Self { status: error.status() } }
            }
        )+
    };
}

raised_from!(
    WatsonxAudioError,
    WatsonxEmbeddingError,
    WatsonxPassthroughError,
    WatsonxRerankError
);

impl From<Raised> for Box<HelperError> {
    fn from(raised: Raised) -> Self {
        Self::new(HelperError {
            class: "WatsonxError".to_owned(),
            status: raised.status,
            error_type: Some(None),
            param: Some(None),
            code: Some(None),
            message: String::new(),
        })
    }
}
