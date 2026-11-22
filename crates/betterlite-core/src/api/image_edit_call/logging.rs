use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::api::image_contracts::records::ImageParams;

// The five values image_edit hands Logging.update_from_kwargs, before the bedrock, stability and BFL branches merge their non-defaults back in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreCallRecord {
    pub model: String,
    pub user: Option<String>,
    pub optional_params: ImageParams,
    pub litellm_params: ImageParams,
    pub custom_llm_provider: String,
}

// The caller's logging object. image_edit reaches it once per call, and only on the configured path.
pub trait ImageEditLogger: Send {
    fn update_from_kwargs(&mut self, record: PreCallRecord) -> Result<(), PrePostError>;
}
