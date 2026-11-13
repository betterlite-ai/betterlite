#![cfg(test)]

mod attribute_truncation;
mod bucket_aggregation;
mod bucket_order;
mod budget_gauges;
mod callback_logger_interface;
mod credential_isolation;
mod drain_behavior;
mod drain_callback_interleave;
mod envelope_timing;
mod flush_behavior;
mod gzip_headers;
mod init_validation;
mod interval_ms;
mod metric_record;
mod periodic_flush;
mod pooled_transport;
mod post_stop_callback;
mod region_resolution;
mod requeue_and_overflow;
mod response_classification;
mod stop_safety;
mod support;
