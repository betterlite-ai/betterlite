mod crates;
mod diff_budget;
pub(crate) mod expect_budget;
mod file_size;
pub(crate) mod globs;
mod lint_table;
mod scope;
mod tests;
mod tools;

use crate::development::Development;
use crate::error::GateError;
use crate::facts::Facts;
use crate::spec::catalog::BuiltinId;
use crate::spec::task::Task;
use crate::validate::validate;

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
        BuiltinId::ExpectBudget => Ok(expect_budget::check(
            context.task.budget.max_new_expects,
            &context.facts.added_rust_lines,
        )),
        BuiltinId::FileSize => file_size::check(context),
        BuiltinId::Crates => crates::check(&context.development.root),
    }
}
