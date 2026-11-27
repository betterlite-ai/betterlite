use betterlite_bedrock::image_edits::response::EditReply;
use betterlite_bedrock::image_models::response::ModelResponse;

pub mod call;
pub mod error;
pub mod execute;
#[cfg(any(test, feature = "testing"))]
pub mod test_support;

mod tests;

// B010's arm carries no data because OperationRequest is built before a generation or edit call reaches its leaf
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BedrockImageRequest;

// One result per image leaf; a leaf's own record travels through unchanged, boxed to keep the enum small
pub enum BedrockImageResult {
    Generate(Box<ModelResponse>),
    Edit(Box<EditReply>),
}

// The prepared arm stays a marker: the dispatcher answers with the leaf's own record
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BedrockImagePrepared;
