#![expect(
    clippy::disallowed_types,
    reason = "Provider responses and tool inputs are opaque JSON that the hook reads and annotates without modelling every provider field"
)]

pub mod execution;
pub mod response;
pub mod settings;
pub mod tool_calls;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::Value;

use self::execution::SkillFiles;
use self::execution::SkillsSandbox;
use self::response::GeneratedFile;
use self::settings::HookSettings;

pub struct SkillsInjectionHook<PromptHandler, Sandbox> {
    pub prompt_handler: PromptHandler,
    pub executor: Sandbox,
    pub settings: HookSettings,
}

impl<PromptHandler: Sync, Sandbox: SkillsSandbox + Sync>
    SkillsInjectionHook<PromptHandler, Sandbox>
{
    pub async fn execute_code(
        &self,
        code: &str,
        skill_files: &SkillFiles,
        generated_files: &mut Vec<GeneratedFile>,
    ) -> String {
        let outcome = match self.executor.execute(code, skill_files).await {
            Ok(outcome) => outcome,
            Err(error) => return format!("Code execution failed: {error}"),
        };
        let mut result = outcome.output.unwrap_or_default();
        for file in outcome.files {
            let decoded = match STANDARD.decode(&file.content_base64) {
                Ok(decoded) => decoded,
                Err(error) => return format!("Code execution failed: {error}"),
            };
            result.push_str("\n\nGenerated file: ");
            result.push_str(&file.name);
            generated_files.push(GeneratedFile {
                name: file.name,
                mime_type: file.mime_type,
                content_base64: file.content_base64,
                size: decoded.len(),
            });
        }
        if let Some(error) = outcome.error {
            result.push_str("\n\nError: ");
            result.push_str(&error);
        }
        if result.is_empty() {
            return "Code executed successfully".to_owned();
        }
        result
    }

    pub async fn execute_skill_tool(
        &self,
        _tool_name: &str,
        _tool_input: &Value,
        skill_files: &SkillFiles,
        generated_files: &mut Vec<GeneratedFile>,
    ) -> String {
        let Some(main_module) = execution::select_skill_module(skill_files) else {
            return self
                .execute_code(
                    execution::NO_SKILL_MODULE_CODE,
                    skill_files,
                    generated_files,
                )
                .await;
        };
        let import_path = main_module.replace('/', ".").replace(".py", "");
        let code = execution::SKILL_EXECUTION_TEMPLATE.replace("__IMPORT_PATH__", &import_path);
        self.execute_code(&code, skill_files, generated_files).await
    }
}
