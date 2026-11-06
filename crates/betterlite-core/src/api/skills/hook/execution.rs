use std::fmt::Display;
use std::future::Future;

use indexmap::IndexMap;

pub type SkillFiles = IndexMap<String, Vec<u8>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxFile {
    pub name: String,
    pub mime_type: String,
    pub content_base64: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SandboxOutcome {
    pub output: Option<String>,
    pub error: Option<String>,
    pub files: Vec<SandboxFile>,
}

pub trait SkillsSandbox {
    type Error: Display;

    fn execute(
        &self,
        code: &str,
        skill_files: &SkillFiles,
    ) -> impl Future<Output = Result<SandboxOutcome, Self::Error>> + Send;
}

pub const NO_SKILL_MODULE_CODE: &str = "
print('No executable skill module found')
";

pub const SKILL_EXECUTION_TEMPLATE: &str = "
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

#[must_use]
pub fn select_skill_module(skill_files: &SkillFiles) -> Option<&str> {
    let python_modules: Vec<&str> = skill_files
        .keys()
        .filter(|path| path.strip_suffix(".py").is_some() && !path.ends_with("__init__.py"))
        .map(String::as_str)
        .collect();
    let preferred = python_modules.iter().find(|path| {
        let lowered = path.to_lowercase();
        lowered.contains("builder") || lowered.contains("creator") || lowered.contains("generator")
    });
    preferred
        .copied()
        .or_else(|| python_modules.first().copied())
}
