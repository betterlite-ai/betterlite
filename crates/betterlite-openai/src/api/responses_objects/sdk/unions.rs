use betterlite_models::decisions::Json;

union! { OutputMessageContentValue {
    OutputText(Box<super::text::OutputText>),
    OutputRefusal(Box<super::text::OutputRefusal>),
} }

union! { OutputTextAnnotationsValue {
    FileCitation(Box<super::text::FileCitation>),
    URLCitation(Box<super::text::URLCitation>),
    ContainerFileCitation(Box<super::text::ContainerFileCitation>),
    FilePath(Box<super::text::FilePath>),
} }

union! { FileSearchResultAttributeValue {
    Text(String),
    Number(super::Number),
    Boolean(bool),
} }

union! { FunctionCallCallerValue {
    CallerDirect(Box<super::output::CallerDirect>),
    CallerProgram(Box<super::output::CallerProgram>),
} }

union! { FunctionCallOutputOutputValue {
    InputText(Box<super::output::InputText>),
    InputImage(Box<super::output::InputImage>),
    InputFile(Box<super::output::InputFile>),
} }

union! { FunctionResult {
    Text(String),
    List(Vec<FunctionCallOutputOutputValue>),
} }

union! { WebSearchActionValue {
    ActionSearch(Box<super::search::ActionSearch>),
    ActionOpenPage(Box<super::search::ActionOpenPage>),
    ActionFind(Box<super::search::ActionFind>),
} }

union! { ComputerActionValue {
    ActionClick(Box<super::computer::ActionClick>),
    ActionDoubleClick(Box<super::computer::ActionDoubleClick>),
    ActionDrag(Box<super::computer::ActionDrag>),
    ActionKeypress(Box<super::computer::ActionKeypress>),
    ActionMove(Box<super::computer::ActionMove>),
    ActionScreenshot(Box<super::computer::ActionScreenshot>),
    ActionScroll(Box<super::computer::ActionScroll>),
    ActionType(Box<super::computer::ActionType>),
    ActionWait(Box<super::computer::ActionWait>),
} }

union! { SearchOutputToolsValue {
    FunctionTool(Box<super::output::FunctionTool>),
    OutputFileSearch(Box<super::search::OutputFileSearch>),
    OutputComputer(Box<super::computer::OutputComputer>),
    ComputerUsePreview(Box<super::computer::ComputerUsePreview>),
    OutputWebSearch(Box<super::search::OutputWebSearch>),
    Mcp(Box<super::tools::Mcp>),
    CodeInterpreter(Box<super::tools::CodeInterpreter>),
    ProgrammaticCalling(Box<super::tools::ProgrammaticCalling>),
    Image(Box<super::tools::Image>),
    LocalShell(Box<super::tools::LocalShell>),
    Shell(Box<super::shell::Shell>),
    Custom(Box<super::output::Custom>),
    Namespace(Box<super::tools::Namespace>),
    Search(Box<super::tools::Search>),
    WebSearchPreview(Box<super::search::WebSearchPreview>),
    ApplyPatch(Box<super::shell::ApplyPatch>),
} }

union! { OutputFileSearchFiltersValue {
    ComparisonFilter(Box<super::tools::ComparisonFilter>),
    CompoundFilter(Box<super::tools::CompoundFilter>),
} }

union! { ComparisonFilterValueValue {
    Text(String),
    Number(super::Number),
} }

union! { ComparisonFilterValueValueSet {
    Text(String),
    Number(super::Number),
    Boolean(bool),
    List(Vec<ComparisonFilterValueValue>),
} }

union! { CompoundFilterFiltersValue {
    ComparisonFilter(Box<super::tools::ComparisonFilter>),
    Json(Box<Json>),
} }

union! { McpAllowedToolsValue {
    List(Vec<String>),
    McpToolFilter(Box<super::tools::McpToolFilter>),
} }

union! { McpRequireApprovalValue {
    McpApprovalFilter(Box<super::tools::McpApprovalFilter>),
    McpRequireApprovalChoice(Box<super::literals::McpRequireApprovalChoice>),
} }

union! { CodeInterpreterContainerValue {
    Text(String),
    AutoCodeContainer(Box<super::tools::AutoCodeContainer>),
} }

union! { AutoCodeContainerNetworkPValue {
    ContainerNetworkPolicyDisabled(Box<super::tools::ContainerNetworkPolicyDisabled>),
    NetworkAllowlist(Box<super::tools::NetworkAllowlist>),
} }

union! { ShellEnvironmentValue {
    ContainerAuto(Box<super::tools::ContainerAuto>),
    LocalEnvironment(Box<super::shell::LocalEnvironment>),
    ContainerReference(Box<super::shell::ContainerReference>),
} }

union! { ContainerAutoSkillsValue {
    SkillReference(Box<super::tools::SkillReference>),
    InlineSkill(Box<super::tools::InlineSkill>),
} }

union! { CustomFormatValue {
    Text(Box<super::output::Text>),
    Grammar(Box<super::output::Grammar>),
} }

union! { NamespaceToolsValue {
    NamedFunctionTool(Box<super::tools::NamedFunctionTool>),
    Custom(Box<super::output::Custom>),
} }

union! { CodeInterpreterCallOutputValue {
    OutputLogs(Box<super::output::OutputLogs>),
    OutputImage(Box<super::output::OutputImage>),
} }

union! { OutputShellEnvironmentValue {
    OutputLocalEnvironment(Box<super::shell::OutputLocalEnvironment>),
    ContainerReference(Box<super::shell::ContainerReference>),
} }

union! { ShellResultOutcomeValue {
    OutputOutcomeTimeout(Box<super::shell::OutputOutcomeTimeout>),
    OutputOutcomeExit(Box<super::shell::OutputOutcomeExit>),
} }

union! { OutputApplyPatchOperationValue {
    OperationCreateFile(Box<super::shell::OperationCreateFile>),
    OperationDeleteFile(Box<super::shell::OperationDeleteFile>),
    OperationUpdateFile(Box<super::shell::OperationUpdateFile>),
} }

union! { SdkOutput {
    OutputMessage(Box<super::text::OutputMessage>),
    FileSearch(Box<super::search::FileSearch>),
    FunctionCall(Box<super::output::FunctionCall>),
    FunctionCallOutput(Box<super::output::FunctionCallOutput>),
    WebSearch(Box<super::search::WebSearch>),
    Computer(Box<super::computer::Computer>),
    ComputerOutput(Box<super::computer::ComputerOutput>),
    Reasoning(Box<super::text::Reasoning>),
    Program(Box<super::output::Program>),
    ProgramOutput(Box<super::output::ProgramOutput>),
    SearchCall(Box<super::tools::SearchCall>),
    SearchOutput(Box<super::tools::SearchOutput>),
    AdditionalTools(Box<super::output::AdditionalTools>),
    Compaction(Box<super::output::Compaction>),
    ImageCall(Box<super::output::ImageCall>),
    CodeInterpreterCall(Box<super::output::CodeInterpreterCall>),
    LocalShellCall(Box<super::output::LocalShellCall>),
    LocalShellCallOutput(Box<super::output::LocalShellCallOutput>),
    OutputShell(Box<super::shell::OutputShell>),
    ShellOutput(Box<super::shell::ShellOutput>),
    OutputApplyPatch(Box<super::shell::OutputApplyPatch>),
    ApplyPatchOutput(Box<super::shell::ApplyPatchOutput>),
    McpCall(Box<super::mcp::McpCall>),
    McpListTools(Box<super::mcp::McpListTools>),
    McpApprovalRequest(Box<super::mcp::McpApprovalRequest>),
    McpApprovalResponse(Box<super::mcp::McpApprovalResponse>),
    CustomCall(Box<super::output::CustomCall>),
    CustomCallOutput(Box<super::output::CustomCallOutput>),
} }
