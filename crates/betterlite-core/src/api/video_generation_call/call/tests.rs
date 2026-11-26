#![cfg(test)]

mod support;

mod async_dispatch;
mod config_missing;
mod default_model;
mod error_mapping;
mod forwarded_arguments;
mod logging_update;
mod mock_response;
mod selection_unavailable;
mod suspension;
mod timeout_fallback;
mod unknown_provider;
mod unsupported_provider;

mod transport_adapter;
