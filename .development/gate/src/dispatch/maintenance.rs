mod tests;

use std::path::Path;

use crate::error::GateError;

pub(super) fn check(git_common_dir: &Path, task: &str) -> Result<(), GateError> {
    let marker = git_common_dir.join("betterlite/maintenance");
    if marker.try_exists().map_err(|source| GateError::Read {
        path: marker.clone(),
        source,
    })? {
        return Err(GateError::Dispatch {
            task: task.to_owned(),
            reason: format!(
                "maintenance is active ({}); no agents will start",
                marker.display()
            ),
        });
    }
    Ok(())
}
