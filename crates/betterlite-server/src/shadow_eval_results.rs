pub mod access;
pub mod error;
pub mod postgres;
pub mod query;
pub mod service;
pub mod store;
mod tests;

// the four shadow eval paths both route modules register and the route-match gate admits
pub const LIST_PATH: &str = "/auto_router/shadow_eval";
pub const START_PATH: &str = "/auto_router/shadow_eval/start";
pub const DETAIL_PATH: &str = "/auto_router/shadow_eval/{job_id}";
pub const STOP_PATH: &str = "/auto_router/shadow_eval/{job_id}/stop";
pub const SHADOW_EVAL_ROUTES: [&str; 4] = [LIST_PATH, START_PATH, DETAIL_PATH, STOP_PATH];
