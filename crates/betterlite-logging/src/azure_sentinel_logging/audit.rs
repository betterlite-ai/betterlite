use crate::audit::payload::StandardAuditLogPayload as SharedAuditLogPayload;

// B4002 and its oracle keep this path after the payload moved to the shared audit module
pub type StandardAuditLogPayload = SharedAuditLogPayload;
