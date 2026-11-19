use crate::auth::auth_identity::AuthContext;
use crate::tracing_routes::access::{TraceAuth, TraceRole};

use super::error::RoiConnectionsError;

pub const READ_FORBIDDEN: &str = "Only proxy admins can access the ROI Calculator.";
pub const WRITE_FORBIDDEN: &str = "Only proxy admins can change ROI Calculator settings.";
pub const READ_ROLES: &[TraceRole] = &[TraceRole::Admin, TraceRole::AdminViewOnly];
pub const WRITE_ROLES: &[TraceRole] = &[TraceRole::Admin];

// TraceAuth already maps the master key and LiteLLM's two proxy admin roles onto its roles.
pub fn ensure_admin(
    caller: Option<&AuthContext>,
    allowed: &[TraceRole],
    forbidden: &'static str,
) -> Result<(), RoiConnectionsError> {
    let Some(context) = caller else {
        return Err(RoiConnectionsError::Unauthenticated);
    };
    let role = TraceAuth::from_principal(&context.principal).role;
    if allowed.contains(&role) {
        return Ok(());
    }
    Err(RoiConnectionsError::Forbidden(forbidden))
}
