pub mod alias;
pub mod budgets;
pub mod claude;
pub mod context;
pub mod router_alias;
pub mod router_settings;
pub mod sso;

mod tests;

// A bare exception during auth becomes auth_error with status 401 and param "None".
pub const AUTH_ERROR_TYPE: &str = "auth_error";
