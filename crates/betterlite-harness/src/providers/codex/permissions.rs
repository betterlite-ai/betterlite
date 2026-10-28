use crate::types::PermissionMode;

pub(super) fn permission_args(
    permissions: PermissionMode,
    is_container: bool,
    native_session_id: Option<&str>,
) -> Vec<String> {
    if permissions != PermissionMode::ReadOnly && is_container {
        return vec!["--dangerously-bypass-approvals-and-sandbox".to_owned()];
    }
    let mode = match permissions {
        PermissionMode::ReadOnly => "read-only",
        PermissionMode::Full | PermissionMode::Ask | PermissionMode::Edit => "workspace-write",
    };
    if native_session_id.is_some() {
        vec!["-c".to_owned(), format!("sandbox_mode=\"{mode}\"")]
    } else {
        vec!["--sandbox".to_owned(), mode.to_owned()]
    }
}
