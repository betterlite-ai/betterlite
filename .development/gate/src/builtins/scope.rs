use crate::builtins::Context;
use crate::builtins::globs::glob_set;
use crate::error::GateError;

pub(crate) fn check(context: &Context<'_>) -> Result<Vec<String>, GateError> {
    let deviation_log = [format!("docs/deviations/{}.yaml", context.task.id)];
    let allowed_patterns = [
        context.task.scope.paths.as_slice(),
        context
            .development
            .catalog
            .settings
            .always_in_scope
            .as_slice(),
        deviation_log.as_slice(),
    ]
    .concat();
    let allowed = glob_set(&allowed_patterns)?;
    Ok(context
        .facts
        .changed
        .iter()
        .filter(|path| !allowed.is_match(path))
        .map(|path| format!("outside the task scope: {path}"))
        .collect())
}
