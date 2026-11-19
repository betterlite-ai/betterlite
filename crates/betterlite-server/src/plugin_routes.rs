pub mod claim;
pub mod config;
pub mod error;
pub(crate) mod fernet;
pub mod headers;
pub mod proxy;
pub mod router;
pub mod routes;

#[cfg(test)]
#[path = "plugin_routes/tests/support.rs"]
mod test_support;

#[cfg(test)]
mod tests;
