use crate::auth::auth_identity::AuthContext;
use crate::tracing_routes::access::TraceAuth;
use crate::tracing_routes::access::TraceRole;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallerAccess {
    FullAdmin,
    AdminView,
    Limited,
}

// LiteLLM lets a proxy admin and a proxy admin viewer read general settings, and redacts secret values for the viewer
#[must_use]
pub fn access_of(caller: &AuthContext) -> CallerAccess {
    match TraceAuth::from_principal(&caller.principal).role {
        TraceRole::Admin => CallerAccess::FullAdmin,
        TraceRole::AdminViewOnly => CallerAccess::AdminView,
        TraceRole::Other => CallerAccess::Limited,
    }
}
