pub mod completion;
pub mod error;
pub mod model_info;
pub mod passthrough;

mod tests;

use betterlite_openai::hosted_vllm::chat::ChatConfig;

// VLLMConfig is an empty HostedVLLMChatConfig subclass, so the vllm SDK keeps the OpenAI params (llms/vllm/completion/transformation.py L7-L13)
pub type VllmConfig = ChatConfig;
