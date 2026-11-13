use crate::api::responses_shapes::FunctionCallStatus;

record! { Shell: ShellTag = "shell" {
    allowed_callers: Option<Vec<super::literals::AllowedCaller>>,
    environment: Option<super::unions::ShellEnvironmentValue>,
} }
record! { LocalEnvironment: LocalEnvironmentTag = "local" {
    skills: Option<Vec<super::tools::LocalSkill>>,
} }
record! { ContainerReference: ContainerReferenceTag = "container_reference" {
    container_id: String,
} }
record! { ApplyPatch: ApplyPatchTag = "apply_patch" {
    allowed_callers: Option<Vec<super::literals::AllowedCaller>>,
} }
record! { OutputShell: OutputShellTag = "shell_call" {
    id: String,
    action: ShellAction,
    call_id: String,
    environment: Option<super::unions::OutputShellEnvironmentValue>,
    status: FunctionCallStatus,
    caller: Option<super::unions::FunctionCallCallerValue>,
    created_by: Option<String>,
} }
record! { ShellAction {
    commands: Vec<String>,
    max_output_length: Option<i64>,
    timeout_ms: Option<i64>,
} }
record! { OutputLocalEnvironment: OutputLocalEnvironmentTag = "local" {} }
record! { ShellOutput: ShellOutputTag = "shell_call_output" {
    id: String,
    call_id: String,
    max_output_length: Option<i64>,
    output: Vec<ShellResult>,
    status: FunctionCallStatus,
    caller: Option<super::unions::FunctionCallCallerValue>,
    created_by: Option<String>,
} }
record! { ShellResult {
    outcome: super::unions::ShellResultOutcomeValue,
    stderr: String,
    stdout: String,
    created_by: Option<String>,
} }
record! { OutputOutcomeTimeout: OutputOutcomeTimeoutTag = "timeout" {} }
record! { OutputOutcomeExit: OutputOutcomeExitTag = "exit" {
    exit_code: i64,
} }
record! { OutputApplyPatch: OutputApplyPatchTag = "apply_patch_call" {
    id: String,
    call_id: String,
    operation: super::unions::OutputApplyPatchOperationValue,
    status: super::literals::OutputApplyPatchStatusChoice,
    caller: Option<super::unions::FunctionCallCallerValue>,
    created_by: Option<String>,
} }
record! { OperationCreateFile: OperationCreateFileTag = "create_file" {
    diff: String,
    path: String,
} }
record! { OperationDeleteFile: OperationDeleteFileTag = "delete_file" {
    path: String,
} }
record! { OperationUpdateFile: OperationUpdateFileTag = "update_file" {
    diff: String,
    path: String,
} }
record! { ApplyPatchOutput: ApplyPatchOutputTag = "apply_patch_call_output" {
    id: String,
    call_id: String,
    status: super::literals::ApplyPatchOutputStatusChoice,
    caller: Option<super::unions::FunctionCallCallerValue>,
    created_by: Option<String>,
    output: Option<String>,
} }
