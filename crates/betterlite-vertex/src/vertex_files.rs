pub mod auth;
pub mod error;
pub mod naming;
pub mod records;
pub mod selection;
pub mod targets;

use betterlite_openai::files::records::CreatePurpose;
use betterlite_prompts::media_metadata::FileData;

// the create-file request plus the deployment values the Vertex transforms read from litellm_params
pub struct FileRequest<'file> {
    pub file: &'file FileData,
    pub purpose: CreatePurpose,
    pub deployment_model: Option<&'file str>,
    pub passthrough: bool,
    pub custom_endpoint: bool,
}
