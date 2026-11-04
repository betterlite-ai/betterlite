use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

strings! { IncompleteReason { MaxOutputTokens = "max_output_tokens", ContentFilter = "content_filter" } }
record! { IncompleteDetails { reason: Option<IncompleteReason> } }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedTokensDetails {
    pub text_tokens: Option<i64>,
    pub audio_tokens: Option<i64>,
    pub image_tokens: Option<i64>,
}

union! { ResponseTools {
    Definitions(Vec<super::unions::SearchOutputToolsValue>),
    FunctionCalls(Vec<super::output::FunctionCall>),
    Unknown(Vec<BTreeMap<String, Json>>),
} }

strings! { ChoiceMode { None = "none", Auto = "auto", Required = "required" } }
strings! { AllowedMode { Auto = "auto", Required = "required" } }
strings! { NamedKind { Function = "function", Custom = "custom" } }
strings! { BuiltinKind {
    FileSearch = "file_search", WebSearchPreview = "web_search_preview",
    Computer = "computer", ComputerUsePreview = "computer_use_preview", ComputerUse = "computer_use",
    WebSearchPreviewDated = "web_search_preview_2025_03_11", ImageGeneration = "image_generation",
    CodeInterpreter = "code_interpreter", Programmatic = "programmatic_tool_calling",
    ApplyPatch = "apply_patch", Shell = "shell",
} }
record! { NamedChoice { r#type: NamedKind, name: String } }
record! { BuiltinChoice { r#type: BuiltinKind } }
record! { AllowedChoice: AllowedChoiceTag = "allowed_tools" {
    mode: AllowedMode,
    tools: Vec<BTreeMap<String, Json>>,
} }
record! { McpChoice: McpChoiceTag = "mcp" {
    server_label: String,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "serde_with::rust::double_option")]
    name: Option<Option<String>>,
} }
union! { ToolChoice {
    Mode(ChoiceMode),
    Named(NamedChoice),
    Builtin(BuiltinChoice),
    Mcp(McpChoice),
    Allowed(AllowedChoice),
} }
