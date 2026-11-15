pub(crate) mod case_ignores;
mod crates;
mod diff_budget;
pub(crate) mod expect_budget;
pub(crate) mod file_size;
pub(crate) mod globs;
mod lint_table;
pub(crate) mod module_tree;
mod scope;
mod tests;
mod tools;
mod workspace;

use crate::development::Development;
use crate::error::GateError;
use crate::facts::Facts;
use crate::spec::catalog::BuiltinId;
use crate::spec::litellm::Litellm;
use crate::spec::task::Task;
use crate::validate::validate;
use crate::yaml::read_yaml;

pub(crate) struct Context<'run> {
    pub(crate) development: &'run Development,
    pub(crate) task: &'run Task,
    pub(crate) facts: &'run Facts,
}

pub(crate) fn run_builtin(id: BuiltinId, context: &Context<'_>) -> Result<Vec<String>, GateError> {
    match id {
        BuiltinId::TaskFile => validate(&context.development.root),
        BuiltinId::Tools => tools::check(context.development),
        BuiltinId::Scope => scope::check(context),
        BuiltinId::LintTable => lint_table::check(context),
        BuiltinId::DiffBudget => diff_budget::check(context),
        BuiltinId::ExpectBudget => {
            let litellm: Litellm =
                read_yaml(&context.development.root.join(".development/litellm.yaml"))?;
            Ok(expect_budget::check(
                context.task.budget.max_new_expects,
                &format!("{}/blob/{}/", litellm.repository, litellm.commit),
                &context.facts.added_rust_lines,
                &context.facts.added_python_lines,
                &context.facts.added_helper_lines,
            ))
        }
        BuiltinId::FileSize => file_size::check(context),
        BuiltinId::Crates => crates::check(&context.development.root),
        BuiltinId::ModuleTree => module_tree::check(context),
        BuiltinId::CaseIgnores => case_ignores::check(context),
    }
}
