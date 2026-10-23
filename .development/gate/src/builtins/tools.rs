use crate::development::Development;
use crate::error::GateError;
use crate::process::check_command;

pub(crate) fn check(development: &Development) -> Result<Vec<String>, GateError> {
    let mut problems = Vec::new();
    for tool in &development.catalog.tools {
        let output = check_command("bash")
            .arg("-c")
            .arg(&tool.version_command)
            .current_dir(&development.root)
            .output()
            .map_err(|source| GateError::Spawn {
                program: tool.version_command.clone(),
                source,
            })?;
        let stdout = String::from_utf8(output.stdout).map_err(|source| GateError::Utf8 {
            what: tool.version_command.clone(),
            source,
        })?;
        let first = stdout.lines().next().unwrap_or_default();
        if !output.status.success() || !first.contains(&tool.expect) {
            problems.push(format!(
                "{}: expected '{}', found '{first}'",
                tool.name, tool.expect
            ));
        }
    }
    Ok(problems)
}
