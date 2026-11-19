#![cfg(test)]

mod fixtures;
mod support;

mod error_attributes;
mod expected_client_error;
mod management_failure;
mod management_success;
mod post_call_failure;
mod post_call_failure_absent_parent;
mod post_call_failure_guardrails;
mod post_call_failure_preprocessing;
mod post_call_success;
mod post_call_success_context;
mod post_call_success_dedupe;
mod post_call_success_dynamic;
mod post_call_success_preprocessing;
mod preprocessing;
mod service_absent_parent;
mod service_attributes;
mod service_datastore;
mod service_event_metadata;
mod service_failure_error;
mod service_failure_timing;
mod service_span;
mod timing;
