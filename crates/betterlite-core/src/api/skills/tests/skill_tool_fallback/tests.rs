#![cfg(test)]

use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::Mutex;

use rstest::rstest;

use crate::api::skills::hook::SkillsInjectionHook;
use crate::api::skills::hook::execution::{SandboxOutcome, SkillFiles, SkillsSandbox};
use crate::api::skills::hook::settings::{HookKwargs, HookSettings};

struct FakeSandbox {
    outcome: SandboxOutcome,
    calls: Mutex<Vec<String>>,
}

impl SkillsSandbox for FakeSandbox {
    type Error = Infallible;

    fn execute(
        &self,
        code: &str,
        _skill_files: &SkillFiles,
    ) -> impl Future<Output = Result<SandboxOutcome, Self::Error>> + Send {
        self.calls.lock().unwrap().push(code.to_owned());
        ready(Ok(self.outcome.clone()))
    }
}

fn hook(outcome: SandboxOutcome) -> SkillsInjectionHook<(), FakeSandbox> {
    SkillsInjectionHook {
        prompt_handler: (),
        executor: FakeSandbox {
            outcome,
            calls: Mutex::new(Vec::new()),
        },
        settings: HookSettings::from(HookKwargs::default()),
    }
}

fn skill_files(paths: &[&str]) -> SkillFiles {
    let mut files = SkillFiles::new();
    for path in paths {
        files.insert((*path).to_owned(), Vec::new());
    }
    files
}

#[rstest]
#[case::no_files(&[])]
#[case::only_init(&["core/__init__.py"])]
#[case::no_python(&["core/notes.txt"])]
#[tokio::test]
async fn skill_tool_fallback_prints_the_litellm_message(#[case] paths: &[&str]) {
    let hook = hook(SandboxOutcome::default());
    let mut generated = Vec::new();
    hook.execute_skill_tool(
        "litellm_skill_gif",
        &serde_json::from_str("{}").unwrap(),
        &skill_files(paths),
        &mut generated,
    )
    .await;
    let calls = hook.executor.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0], "\nprint('No executable skill module found')\n");
}

#[tokio::test]
async fn skill_tool_fallback_returns_the_executor_result_for_the_fallback_code() {
    let hook = hook(SandboxOutcome {
        output: Some("No executable skill module found".to_owned()),
        error: None,
        files: Vec::new(),
    });
    let mut generated = Vec::new();
    let result = hook
        .execute_skill_tool(
            "litellm_skill_gif",
            &serde_json::from_str("{}").unwrap(),
            &SkillFiles::new(),
            &mut generated,
        )
        .await;
    assert_eq!(result, "No executable skill module found");
}
