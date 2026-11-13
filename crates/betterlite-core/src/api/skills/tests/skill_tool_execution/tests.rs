#![cfg(test)]

use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::Mutex;

use rstest::rstest;

use crate::api::skills::hook::SkillsInjectionHook;
use crate::api::skills::hook::execution::{
    SandboxOutcome, SkillFiles, SkillsSandbox, select_skill_module,
};
use crate::api::skills::hook::settings::{HookKwargs, HookSettings};

const EXPECTED_TEMPLATE: &str = "
# Auto-generated code to execute skill
import sys
sys.path.insert(0, '/sandbox')

from __IMPORT_PATH__ import *

# Try to find and use a Builder/Creator class
import inspect
module = __import__('__IMPORT_PATH__', fromlist=[''])

for name, obj in inspect.getmembers(module):
    if inspect.isclass(obj) and name != 'object':
        try:
            instance = obj()
            # Try common methods
            if hasattr(instance, 'create'):
                result = instance.create()
            elif hasattr(instance, 'build'):
                result = instance.build()
            elif hasattr(instance, 'generate'):
                result = instance.generate()
            elif hasattr(instance, 'save'):
                instance.save('output.gif')
            print(f'Used {name} class')
            break
        except Exception as e:
            print(f'Error with {name}: {e}')
            continue

# List generated files
import os
for f in os.listdir('.'):
    if f.endswith(('.gif', '.png', '.jpg')):
        print(f'Generated: {f}')
";

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

fn executed_code(hook: &SkillsInjectionHook<(), FakeSandbox>) -> String {
    let calls = hook.executor.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    calls[0].clone()
}

#[tokio::test]
async fn skill_tool_execution_generates_the_litellm_code_for_the_chosen_module() {
    let hook = hook(SandboxOutcome::default());
    let mut generated = Vec::new();
    let result = hook
        .execute_skill_tool(
            "litellm_skill_gif",
            &serde_json::from_str("{}").unwrap(),
            &skill_files(&["core/gif_builder.py"]),
            &mut generated,
        )
        .await;
    assert_eq!(result, "Code executed successfully");
    assert_eq!(
        executed_code(&hook),
        EXPECTED_TEMPLATE.replace("__IMPORT_PATH__", "core.gif_builder")
    );
}

#[tokio::test]
async fn skill_tool_execution_returns_the_executor_output() {
    let hook = hook(SandboxOutcome {
        output: Some("Used GifBuilder class".to_owned()),
        error: None,
        files: Vec::new(),
    });
    let mut generated = Vec::new();
    let result = hook
        .execute_skill_tool(
            "litellm_skill_gif",
            &serde_json::from_str("{}").unwrap(),
            &skill_files(&["core/gif_builder.py"]),
            &mut generated,
        )
        .await;
    assert_eq!(result, "Used GifBuilder class");
}

#[tokio::test]
async fn skill_tool_execution_ignores_the_tool_name_and_input() {
    let files = skill_files(&["core/gif_builder.py"]);
    let first = hook(SandboxOutcome::default());
    let mut generated = Vec::new();
    first
        .execute_skill_tool(
            "litellm_skill_one",
            &serde_json::from_str("{}").unwrap(),
            &files,
            &mut generated,
        )
        .await;
    let second = hook(SandboxOutcome::default());
    second
        .execute_skill_tool(
            "litellm_skill_other",
            &serde_json::from_str(r#"{"style": "flat"}"#).unwrap(),
            &files,
            &mut generated,
        )
        .await;
    assert_eq!(executed_code(&first), executed_code(&second));
}

#[rstest]
#[case::builder_after_plain(&["core/main.py", "core/gif_builder.py"], "core/gif_builder.py")]
#[case::creator(&["core/main.py", "core/creator.py"], "core/creator.py")]
#[case::generator_any_case(&["core/tool.py", "core/Generator.py"], "core/Generator.py")]
#[case::first_plain_module(&["core/helpers.py", "core/main.py"], "core/helpers.py")]
#[case::skips_init(&["core/__init__.py", "core/main.py"], "core/main.py")]
#[case::skips_non_python(&["notes.txt", "core/main.py"], "core/main.py")]
fn skill_tool_execution_selects_the_module(#[case] paths: &[&str], #[case] module_path: &str) {
    let files = skill_files(paths);
    let selected = select_skill_module(&files);
    assert_eq!(selected, Some(module_path));
}

#[tokio::test]
async fn skill_tool_execution_dots_a_nested_module_path() {
    let hook = hook(SandboxOutcome::default());
    let mut generated = Vec::new();
    hook.execute_skill_tool(
        "litellm_skill_gif",
        &serde_json::from_str("{}").unwrap(),
        &skill_files(&["skills/gif/builder.py"]),
        &mut generated,
    )
    .await;
    assert_eq!(
        executed_code(&hook),
        EXPECTED_TEMPLATE.replace("__IMPORT_PATH__", "skills.gif.builder")
    );
}

#[tokio::test]
async fn skill_tool_execution_strips_every_py_suffix_like_python_replace() {
    let hook = hook(SandboxOutcome::default());
    let mut generated = Vec::new();
    hook.execute_skill_tool(
        "litellm_skill_gif",
        &serde_json::from_str("{}").unwrap(),
        &skill_files(&["a.py/builder.py"]),
        &mut generated,
    )
    .await;
    assert_eq!(
        executed_code(&hook),
        EXPECTED_TEMPLATE.replace("__IMPORT_PATH__", "a.builder")
    );
}
