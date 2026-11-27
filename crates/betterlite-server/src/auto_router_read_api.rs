pub mod access;
pub mod error;
pub mod postgres;
pub mod query;
pub mod service;
pub mod store;

mod tests;

// the two auto-router reads the dashboard and a coding agent's status line call
pub const BENCHMARKS_PATH: &str = "/auto_router/benchmarks";
pub const SESSION_PATH: &str = "/auto_router/session";
pub const READ_PATHS: [&str; 2] = [BENCHMARKS_PATH, SESSION_PATH];

// _require_admin_viewer's wording for the benchmarks read
pub const VIEW_ACTION: &str = "view auto-router benchmarks across the deployment";
