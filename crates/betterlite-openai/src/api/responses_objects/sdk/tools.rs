use crate::api::responses_shapes::FunctionCallStatus;
use betterlite_models::decisions::Json;
use std::collections::BTreeMap;

record! { SearchCall: SearchCallTag = "tool_search_call" {
    id: String,
    #[serde(deserialize_with = "Json::deserialize")]
    arguments: Json,
    call_id: Option<String>,
    execution: super::literals::SearchCallExecutionChoice,
    status: FunctionCallStatus,
    created_by: Option<String>,
} }
record! { SearchOutput: SearchOutputTag = "tool_search_output" {
    id: String,
    call_id: Option<String>,
    execution: super::literals::SearchCallExecutionChoice,
    status: FunctionCallStatus,
    tools: Vec<super::unions::SearchOutputToolsValue>,
    created_by: Option<String>,
} }
record! { ComparisonFilter {
    key: String,
    r#type: super::literals::ComparisonFilterTypeChoice,
    value: super::unions::ComparisonFilterValueValueSet,
} }
record! { CompoundFilter {
    filters: Vec<super::unions::CompoundFilterFiltersValue>,
    r#type: super::literals::CompoundFilterTypeChoice,
} }
record! { Mcp: McpTag = "mcp" {
    server_label: String,
    allowed_callers: Option<Vec<super::literals::AllowedCaller>>,
    allowed_tools: Option<super::unions::McpAllowedToolsValue>,
    authorization: Option<String>,
    connector_id: Option<super::literals::McpConnectorIdChoice>,
    defer_loading: Option<bool>,
    headers: Option<BTreeMap<String, String>>,
    require_approval: Option<super::unions::McpRequireApprovalValue>,
    server_description: Option<String>,
    server_url: Option<String>,
    tunnel_id: Option<String>,
} }
record! { McpToolFilter {
    read_only: Option<bool>,
    tool_names: Option<Vec<String>>,
} }
record! { McpApprovalFilter {
    always: Option<McpToolFilter>,
    never: Option<McpToolFilter>,
} }
record! { CodeInterpreter: CodeInterpreterTag = "code_interpreter" {
    container: super::unions::CodeInterpreterContainerValue,
    allowed_callers: Option<Vec<super::literals::AllowedCaller>>,
} }
record! { AutoCodeContainer: AutoCodeContainerTag = "auto" {
    file_ids: Option<Vec<String>>,
    memory_limit: Option<super::literals::CodeContainerMemoryChoice>,
    network_policy: Option<super::unions::ContainerNetworkPolicy>,
} }
record! { ContainerNetworkPolicyDisabled: NetworkDisabledTag = "disabled" {} }
record! { NetworkAllowlist: NetworkAllowlistTag = "allowlist" {
    allowed_domains: Vec<String>,
    domain_secrets: Option<Vec<DomainSecret>>,
} }
record! { DomainSecret {
    domain: String,
    name: String,
    value: String,
} }
record! { ProgrammaticCalling: ProgrammaticCallingTag = "programmatic_tool_calling" {} }
record! { Image: ImageTag = "image_generation" {
    action: Option<super::literals::ImageActionChoice>,
    background: Option<super::literals::ImageBackgroundChoice>,
    input_fidelity: Option<super::literals::ImageInputFidelityChoice>,
    input_image_mask: Option<ImageInputImageMask>,
    model: Option<String>,
    moderation: Option<super::literals::ImageModerationChoice>,
    output_compression: Option<i64>,
    output_format: Option<super::literals::ImageOutputFormatChoice>,
    partial_images: Option<i64>,
    quality: Option<super::literals::ImageQualityChoice>,
    size: Option<String>,
} }
record! { ImageInputImageMask {
    file_id: Option<String>,
    image_url: Option<String>,
} }
record! { LocalShell: LocalShellTag = "local_shell" {} }
record! { ContainerAuto: ContainerAutoTag = "container_auto" {
    file_ids: Option<Vec<String>>,
    memory_limit: Option<super::literals::CodeContainerMemoryChoice>,
    network_policy: Option<super::unions::ContainerNetworkPolicy>,
    skills: Option<Vec<super::unions::ContainerAutoSkillsValue>>,
} }
record! { SkillReference: SkillReferenceTag = "skill_reference" {
    skill_id: String,
    version: Option<String>,
} }
record! { InlineSkill: InlineSkillTag = "inline" {
    description: String,
    name: String,
    source: InlineSkillSource,
} }
record! { InlineSkillSource: InlineSkillSourceTag = "base64" {
    data: String,
    media_type: super::literals::ApplicationZipKind,
} }
record! { LocalSkill {
    description: String,
    name: String,
    path: String,
} }
record! { Namespace: NamespaceTag = "namespace" {
    description: String,
    name: String,
    tools: Vec<super::unions::NamespaceToolsValue>,
} }
record! { NamedFunctionTool: NamedFunctionToolTag = "function" {
    name: String,
    allowed_callers: Option<Vec<super::literals::AllowedCaller>>,
    defer_loading: Option<bool>,
    description: Option<String>,
    output_schema: Option<BTreeMap<String, Json>>,
    parameters: Option<Json>,
    strict: Option<bool>,
} }
record! { Search: SearchTag = "tool_search" {
    description: Option<String>,
    execution: Option<super::literals::SearchCallExecutionChoice>,
    parameters: Option<Json>,
} }
