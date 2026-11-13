use crate::api::responses_shapes::FunctionCallStatus;
use betterlite_models::decisions::Json;
use std::collections::BTreeMap;

record! { FunctionCall: FunctionCallTag = "function_call" {
    arguments: String,
    call_id: String,
    name: String,
    id: Option<String>,
    caller: Option<super::unions::FunctionCallCallerValue>,
    namespace: Option<String>,
    status: Option<FunctionCallStatus>,
} }
record! { CallerDirect: CallerDirectTag = "direct" {} }
record! { CallerProgram: CallerProgramTag = "program" {
    caller_id: String,
} }
record! { FunctionCallOutput: FunctionCallOutputTag = "function_call_output" {
    id: String,
    call_id: String,
    output: super::unions::FunctionResult,
    status: FunctionCallStatus,
    caller: Option<super::unions::FunctionCallCallerValue>,
    created_by: Option<String>,
    name: Option<String>,
    namespace: Option<String>,
} }
record! { InputText: InputTextTag = "input_text" {
    text: String,
    prompt_cache_breakpoint: Option<PromptCacheBreakpoint>,
} }
record! { PromptCacheBreakpoint {
    mode: super::literals::ExplicitKind,
} }
record! { InputImage: InputImageTag = "input_image" {
    detail: super::literals::InputImageDetailChoice,
    file_id: Option<String>,
    image_url: Option<String>,
    prompt_cache_breakpoint: Option<PromptCacheBreakpoint>,
} }
record! { InputFile: InputFileTag = "input_file" {
    detail: Option<super::literals::InputFileDetailChoice>,
    file_data: Option<String>,
    file_id: Option<String>,
    file_url: Option<String>,
    filename: Option<String>,
    prompt_cache_breakpoint: Option<PromptCacheBreakpoint>,
} }
record! { Program: ProgramTag = "program" {
    id: String,
    call_id: String,
    code: String,
    fingerprint: String,
} }
record! { ProgramOutput: ProgramOutputTag = "program_output" {
    id: String,
    call_id: String,
    result: String,
    status: super::literals::ProgramOutputStatusChoice,
} }
record! { FunctionTool: FunctionToolTag = "function" {
    name: String,
    parameters: Option<BTreeMap<String, Json>>,
    strict: Option<bool>,
    allowed_callers: Option<Vec<super::literals::AllowedCaller>>,
    defer_loading: Option<bool>,
    description: Option<String>,
    output_schema: Option<BTreeMap<String, Json>>,
} }
record! { Custom: CustomTag = "custom" {
    name: String,
    allowed_callers: Option<Vec<super::literals::AllowedCaller>>,
    defer_loading: Option<bool>,
    description: Option<String>,
    format: Option<super::unions::CustomFormatValue>,
} }
record! { Text: TextTag = "text" {} }
record! { Grammar: GrammarTag = "grammar" {
    definition: String,
    syntax: super::literals::GrammarSyntaxChoice,
} }
record! { AdditionalTools: AdditionalToolsTag = "additional_tools" {
    id: String,
    role: super::literals::AdditionalToolsRoleChoice,
    tools: Vec<super::unions::SearchOutputToolsValue>,
} }
record! { Compaction: CompactionTag = "compaction" {
    id: String,
    encrypted_content: String,
    created_by: Option<String>,
} }
record! { ImageCall: ImageCallTag = "image_generation_call" {
    id: String,
    result: Option<String>,
    status: super::literals::ImageCallStatusChoice,
} }
record! { CodeInterpreterCall: CodeInterpreterCallTag = "code_interpreter_call" {
    id: String,
    code: Option<String>,
    container_id: String,
    outputs: Option<Vec<super::unions::CodeInterpreterCallOutputValue>>,
    status: super::literals::InterpreterStatus,
} }
record! { OutputLogs: OutputLogsTag = "logs" {
    logs: String,
} }
record! { OutputImage: OutputImageTag = "image" {
    url: String,
} }
record! { LocalShellCall: LocalShellCallTag = "local_shell_call" {
    id: String,
    action: LocalShellCallAction,
    call_id: String,
    status: FunctionCallStatus,
} }
record! { LocalShellCallAction: LocalShellCallActionTag = "exec" {
    command: Vec<String>,
    env: BTreeMap<String, String>,
    timeout_ms: Option<i64>,
    user: Option<String>,
    working_directory: Option<String>,
} }
record! { LocalShellCallOutput: LocalShellCallOutputTag = "local_shell_call_output" {
    id: String,
    output: String,
    status: Option<FunctionCallStatus>,
} }
record! { CustomCall: CustomCallTag = "custom_tool_call" {
    call_id: String,
    input: String,
    name: String,
    id: Option<String>,
    caller: Option<super::unions::FunctionCallCallerValue>,
    namespace: Option<String>,
} }
record! { CustomCallOutput: CustomCallOutputTag = "custom_tool_call_output" {
    call_id: String,
    output: super::unions::FunctionResult,
    id: String,
    caller: Option<super::unions::FunctionCallCallerValue>,
    status: FunctionCallStatus,
    created_by: Option<String>,
} }
