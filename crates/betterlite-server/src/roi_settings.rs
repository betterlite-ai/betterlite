pub mod auth;
pub mod deployments;
pub mod error;
pub mod patch;
pub mod public;
pub mod repository;
pub mod route;
pub mod state;
pub mod store;

include!("roi_settings/tests/declarations.rs");

mod tests;
