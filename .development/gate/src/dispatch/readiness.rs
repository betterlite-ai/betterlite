mod tests;

use std::fs;
use std::path::Path;

use crate::builtins::globs::glob_set;
use crate::error::GateError;
use crate::spec::task::Task;
use globset::GlobSet;

pub(super) fn check(root: &Path, task: &Task) -> Result<(), GateError> {
    let scope = glob_set(&task.scope.paths)?;
    for path in &task.scope.paths {
        if path.contains(['*', '?', '[', '{'])
            || Path::new(path)
                .extension()
                .is_none_or(|extension| extension != "rs")
            || root.join(path).exists()
        {
            continue;
        }
        let relative = Path::new(path);
        let (Some(parent), Some(name)) = (
            relative.parent(),
            relative.file_stem().and_then(|part| part.to_str()),
        ) else {
            continue;
        };
        if ["lib", "main", "mod", "tests"].contains(&name)
            || path.contains("/tests/")
            || path.contains("/fuzz/")
        {
            continue;
        }
        if reachable(root, parent, name, &scope) {
            continue;
        }
        return Err(GateError::Dispatch {
            task: task.id.clone(),
            reason: format!(
                "{path} is new but its parent module neither declares it nor belongs to scope; fix the integration path before dispatch"
            ),
        });
    }
    Ok(())
}

fn reachable(root: &Path, parent: &Path, name: &str, scope: &GlobSet) -> bool {
    let candidates = [
        parent.with_extension("rs"),
        parent.join("mod.rs"),
        parent.join("lib.rs"),
        parent.join("main.rs"),
    ];
    let existing: Vec<_> = candidates
        .iter()
        .filter(|candidate| root.join(candidate).is_file())
        .collect();
    if existing.is_empty() || candidates.iter().any(|candidate| scope.is_match(candidate)) {
        return true;
    }
    existing.iter().any(|candidate| {
        fs::read_to_string(root.join(candidate)).is_ok_and(|text| {
            text.contains(&format!("mod {name}"))
                || text.contains("#[path")
                || text.contains("include!")
        })
    })
}
