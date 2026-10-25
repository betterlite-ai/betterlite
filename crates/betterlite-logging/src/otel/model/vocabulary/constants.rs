use strum::{Display, IntoStaticStr};

#[path = "constants/attribute_keys.rs"]
pub mod attribute_keys;
#[path = "constants/tests.rs"]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
#[strum(serialize_all = "snake_case")]
pub enum GenAIOperation {
    Chat,
    TextCompletion,
    Embeddings,
    GenerateContent,
    Retrieval,
    CreateAgent,
    InvokeAgent,
    ExecuteTool,
    // the convention names nothing for these, so they take litellm.* vendor values
    #[strum(serialize = "litellm.vector_store_management")]
    VectorStoreManagement,
    #[strum(serialize = "litellm.vector_store_file_management")]
    VectorStoreFileManagement,
    #[strum(serialize = "litellm.responses_management")]
    ResponsesManagement,
    #[strum(serialize = "litellm.moderation")]
    Moderation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
#[strum(serialize_all = "snake_case")]
pub enum GenAIOutputType {
    Text,
    Json,
    Image,
    Speech,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum GenAIProvider {
    Openai,
    Anthropic,
    #[strum(serialize = "aws.bedrock")]
    AwsBedrock,
    #[strum(serialize = "azure.ai.openai")]
    AzureAiOpenai,
    #[strum(serialize = "azure.ai.inference")]
    AzureAiInference,
    #[strum(serialize = "gcp.gemini")]
    GcpGemini,
    #[strum(serialize = "gcp.vertex_ai")]
    GcpVertexAi,
    Cohere,
    MistralAi,
    Deepseek,
    Groq,
    Perplexity,
    XAi,
    #[strum(serialize = "ibm.watsonx.ai")]
    IbmWatsonxAi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, strum::EnumString)]
pub enum MCPMethod {
    #[strum(serialize = "tools/call")]
    ToolsCall,
    #[strum(serialize = "tools/list")]
    ToolsList,
    #[strum(serialize = "prompts/get")]
    PromptsGet,
    #[strum(serialize = "prompts/list")]
    PromptsList,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
#[strum(serialize_all = "lowercase")]
pub enum RpcSystem {
    JsonRpc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
#[strum(serialize_all = "lowercase")]
pub enum NetworkTransport {
    Tcp,
    Udp,
    Quic,
    Unix,
    Pipe,
}
