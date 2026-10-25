#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum GenAIOperation {
    Chat,
    TextCompletion,
    Embeddings,
    GenerateContent,
    Retrieval,
    CreateAgent,
    InvokeAgent,
    ExecuteTool,
    LitellmVectorStoreManagement,
    LitellmVectorStoreFileManagement,
    LitellmResponsesManagement,
    LitellmModeration,
}
impl GenAIOperation {
    pub const CHAT: Self = Self::Chat;
    pub const TEXT_COMPLETION: Self = Self::TextCompletion;
    pub const EMBEDDINGS: Self = Self::Embeddings;
    pub const GENERATE_CONTENT: Self = Self::GenerateContent;
    pub const RETRIEVAL: Self = Self::Retrieval;
    pub const CREATE_AGENT: Self = Self::CreateAgent;
    pub const INVOKE_AGENT: Self = Self::InvokeAgent;
    pub const EXECUTE_TOOL: Self = Self::ExecuteTool;
    pub const LITELLM_VECTOR_STORE_MANAGEMENT: Self = Self::LitellmVectorStoreManagement;
    pub const LITELLM_VECTOR_STORE_FILE_MANAGEMENT: Self = Self::LitellmVectorStoreFileManagement;
    pub const LITELLM_RESPONSES_MANAGEMENT: Self = Self::LitellmResponsesManagement;
    pub const LITELLM_MODERATION: Self = Self::LitellmModeration;
    #[must_use]
    pub const fn value(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::TextCompletion => "text_completion",
            Self::Embeddings => "embeddings",
            Self::GenerateContent => "generate_content",
            Self::Retrieval => "retrieval",
            Self::CreateAgent => "create_agent",
            Self::InvokeAgent => "invoke_agent",
            Self::ExecuteTool => "execute_tool",
            Self::LitellmVectorStoreManagement => "litellm.vector_store_management",
            Self::LitellmVectorStoreFileManagement => "litellm.vector_store_file_management",
            Self::LitellmResponsesManagement => "litellm.responses_management",
            Self::LitellmModeration => "litellm.moderation",
        }
    }
}
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum GenAIOutputType {
    Text,
    Json,
    Image,
    Speech,
}
impl GenAIOutputType {
    pub const TEXT: Self = Self::Text;
    pub const JSON: Self = Self::Json;
    pub const IMAGE: Self = Self::Image;
    pub const SPEECH: Self = Self::Speech;
    #[must_use]
    pub const fn value(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Json => "json",
            Self::Image => "image",
            Self::Speech => "speech",
        }
    }
}
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum GenAIProvider {
    Openai,
    Anthropic,
    AwsBedrock,
    AzureAiOpenai,
    AzureAiInference,
    GcpGemini,
    GcpVertexAi,
    Cohere,
    MistralAi,
    Deepseek,
    Groq,
    Perplexity,
    XAi,
    IbmWatsonxAi,
}
impl GenAIProvider {
    pub const OPENAI: Self = Self::Openai;
    pub const ANTHROPIC: Self = Self::Anthropic;
    pub const AWS_BEDROCK: Self = Self::AwsBedrock;
    pub const AZURE_AI_OPENAI: Self = Self::AzureAiOpenai;
    pub const AZURE_AI_INFERENCE: Self = Self::AzureAiInference;
    pub const GCP_GEMINI: Self = Self::GcpGemini;
    pub const GCP_VERTEX_AI: Self = Self::GcpVertexAi;
    pub const COHERE: Self = Self::Cohere;
    pub const MISTRAL_AI: Self = Self::MistralAi;
    pub const DEEPSEEK: Self = Self::Deepseek;
    pub const GROQ: Self = Self::Groq;
    pub const PERPLEXITY: Self = Self::Perplexity;
    pub const X_AI: Self = Self::XAi;
    pub const IBM_WATSONX_AI: Self = Self::IbmWatsonxAi;
    #[must_use]
    pub const fn value(self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Anthropic => "anthropic",
            Self::AwsBedrock => "aws.bedrock",
            Self::AzureAiOpenai => "azure.ai.openai",
            Self::AzureAiInference => "azure.ai.inference",
            Self::GcpGemini => "gcp.gemini",
            Self::GcpVertexAi => "gcp.vertex_ai",
            Self::Cohere => "cohere",
            Self::MistralAi => "mistral_ai",
            Self::Deepseek => "deepseek",
            Self::Groq => "groq",
            Self::Perplexity => "perplexity",
            Self::XAi => "x_ai",
            Self::IbmWatsonxAi => "ibm.watsonx.ai",
        }
    }
}
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum MCPMethod {
    ToolsCall,
    ToolsList,
    PromptsGet,
    PromptsList,
}
impl MCPMethod {
    pub const TOOLS_CALL: Self = Self::ToolsCall;
    pub const TOOLS_LIST: Self = Self::ToolsList;
    pub const PROMPTS_GET: Self = Self::PromptsGet;
    pub const PROMPTS_LIST: Self = Self::PromptsList;
    #[must_use]
    pub const fn value(self) -> &'static str {
        match self {
            Self::ToolsCall => "tools/call",
            Self::ToolsList => "tools/list",
            Self::PromptsGet => "prompts/get",
            Self::PromptsList => "prompts/list",
        }
    }
}
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum NetworkTransport {
    Tcp,
    Udp,
    Quic,
    Unix,
    Pipe,
}
impl NetworkTransport {
    pub const TCP: Self = Self::Tcp;
    pub const UDP: Self = Self::Udp;
    pub const QUIC: Self = Self::Quic;
    pub const UNIX: Self = Self::Unix;
    pub const PIPE: Self = Self::Pipe;
    #[must_use]
    pub const fn value(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
            Self::Quic => "quic",
            Self::Unix => "unix",
            Self::Pipe => "pipe",
        }
    }
}
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum RpcSystem {
    JsonRpc,
}
impl RpcSystem {
    pub const JSONRPC: Self = Self::JsonRpc;
    #[must_use]
    pub const fn value(self) -> &'static str {
        "jsonrpc"
    }
}
pub const LITELLM_PROXY_REQUEST_SPAN_NAME: &str = "Received Proxy Server Request";
#[cfg(test)]
#[path = "constants/tests.rs"]
mod tests;
