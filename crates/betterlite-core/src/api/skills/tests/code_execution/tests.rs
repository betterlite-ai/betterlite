#![cfg(test)]

use std::future::{Future, ready};
use std::sync::Mutex;

use crate::api::skills::hook::SkillsInjectionHook;
use crate::api::skills::hook::execution::{SandboxFile, SandboxOutcome, SkillFiles, SkillsSandbox};
use crate::api::skills::hook::response::GeneratedFile;
use crate::api::skills::hook::settings::{HookKwargs, HookSettings};

struct FakeSandbox {
    outcome: Result<SandboxOutcome, String>,
    calls: Mutex<Vec<(String, Vec<String>)>>,
}

impl FakeSandbox {
    fn new(outcome: Result<SandboxOutcome, &str>) -> Self {
        Self {
            outcome: outcome.map_err(str::to_owned),
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl SkillsSandbox for FakeSandbox {
    type Error = String;

    fn execute(
        &self,
        code: &str,
        skill_files: &SkillFiles,
    ) -> impl Future<Output = Result<SandboxOutcome, String>> + Send {
        self.calls
            .lock()
            .unwrap()
            .push((code.to_owned(), skill_files.keys().cloned().collect()));
        ready(self.outcome.clone())
    }
}

fn hook(sandbox: FakeSandbox) -> SkillsInjectionHook<(), FakeSandbox> {
    SkillsInjectionHook {
        prompt_handler: (),
        executor: sandbox,
        settings: HookSettings::from(HookKwargs::default()),
    }
}

fn skill_files() -> SkillFiles {
    let mut files = SkillFiles::new();
    files.insert("core/gif_builder.py".to_owned(), b"print(1)".to_vec());
    files
}

#[tokio::test]
async fn code_execution_passes_the_code_and_skill_files_to_the_executor() {
    let hook = hook(FakeSandbox::new(Ok(SandboxOutcome {
        output: Some("hi".to_owned()),
        error: None,
        files: Vec::new(),
    })));
    let mut generated = Vec::new();
    let result = hook
        .execute_code("print(42)", &skill_files(), &mut generated)
        .await;
    assert_eq!(result, "hi");
    assert!(generated.is_empty());
    let calls = hook.executor.calls.lock().unwrap();
    assert_eq!(
        *calls,
        vec![(
            "print(42)".to_owned(),
            vec!["core/gif_builder.py".to_owned()],
        )]
    );
}

#[tokio::test]
async fn code_execution_collects_generated_files_with_the_decoded_size() {
    let hook = hook(FakeSandbox::new(Ok(SandboxOutcome {
        output: Some("rendered".to_owned()),
        error: None,
        files: vec![SandboxFile {
            name: "chart.png".to_owned(),
            mime_type: "image/png".to_owned(),
            content_base64: "aGVsbG8=".to_owned(),
        }],
    })));
    let mut generated = Vec::new();
    let result = hook
        .execute_code("render()", &skill_files(), &mut generated)
        .await;
    assert_eq!(result, "rendered\n\nGenerated file: chart.png");
    assert_eq!(
        generated,
        vec![GeneratedFile {
            name: "chart.png".to_owned(),
            mime_type: "image/png".to_owned(),
            content_base64: "aGVsbG8=".to_owned(),
            size: 5,
        }]
    );
}

#[tokio::test]
async fn code_execution_appends_the_error_after_the_output() {
    let hook = hook(FakeSandbox::new(Ok(SandboxOutcome {
        output: Some("done".to_owned()),
        error: Some("deprecated call".to_owned()),
        files: Vec::new(),
    })));
    let mut generated = Vec::new();
    let result = hook
        .execute_code("run()", &skill_files(), &mut generated)
        .await;
    assert_eq!(result, "done\n\nError: deprecated call");
}

#[tokio::test]
async fn code_execution_answers_success_when_nothing_came_back() {
    let hook = hook(FakeSandbox::new(Ok(SandboxOutcome::default())));
    let mut generated = Vec::new();
    let result = hook
        .execute_code("run()", &skill_files(), &mut generated)
        .await;
    assert_eq!(result, "Code executed successfully");
}

#[tokio::test]
async fn code_execution_answers_success_for_an_empty_output() {
    let hook = hook(FakeSandbox::new(Ok(SandboxOutcome {
        output: Some(String::new()),
        error: None,
        files: Vec::new(),
    })));
    let mut generated = Vec::new();
    let result = hook
        .execute_code("run()", &skill_files(), &mut generated)
        .await;
    assert_eq!(result, "Code executed successfully");
}

#[tokio::test]
async fn code_execution_turns_executor_failures_into_the_result_text() {
    let hook = hook(FakeSandbox::new(Err("container is gone")));
    let mut generated = Vec::new();
    let result = hook
        .execute_code("run()", &skill_files(), &mut generated)
        .await;
    assert_eq!(result, "Code execution failed: container is gone");
    assert!(generated.is_empty());
}

#[tokio::test]
async fn code_execution_turns_a_bad_file_payload_into_the_result_text() {
    let hook = hook(FakeSandbox::new(Ok(SandboxOutcome {
        output: Some("rendered".to_owned()),
        error: None,
        files: vec![SandboxFile {
            name: "chart.png".to_owned(),
            mime_type: "image/png".to_owned(),
            content_base64: "not base64!".to_owned(),
        }],
    })));
    let mut generated = Vec::new();
    let result = hook
        .execute_code("render()", &skill_files(), &mut generated)
        .await;
    assert!(result.starts_with("Code execution failed: "));
    assert!(generated.is_empty());
}
