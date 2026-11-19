#![cfg(test)]

mod config;
mod datadog_exception_handling_behavior;
mod datadog_metadata_merging_behavior;
mod datadog_payload_types_behavior;
mod datadog_proxy_error_extraction_behavior;
mod datadog_proxy_queueing_flush_behavior;
mod datadog_proxy_user_context_behavior;
mod datadog_service_failure_queueing_behavior;
mod datadog_service_success_filtering_behavior;
mod datadog_success_service_types_behavior;
mod datadog_trace_context_attachment_behavior;
mod legacy;
mod standard;
mod support;
mod trace;
