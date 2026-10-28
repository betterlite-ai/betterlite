use crate::context::SessionContext;
use crate::sandbox::Sandbox;
use crate::types::PermissionMode;

pub(super) fn permission_args<S>(context: &SessionContext<S>, resumed: bool) -> Vec<String>
where
    S: Sandbox,
{
    if context.settings.permissions != PermissionMode::ReadOnly && context.sandbox.is_container() {
        return vec!["--dangerously-bypass-approvals-and-sandbox".to_owned()];
    }
    let mode = match context.settings.permissions {
        PermissionMode::ReadOnly => "read-only",
        PermissionMode::Full | PermissionMode::Ask | PermissionMode::Edit => "workspace-write",
    };
    if resumed {
        vec!["-c".to_owned(), format!("sandbox_mode=\"{mode}\"")]
    } else {
        vec!["--sandbox".to_owned(), mode.to_owned()]
    }
}
