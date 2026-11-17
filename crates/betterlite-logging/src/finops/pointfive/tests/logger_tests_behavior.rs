#![cfg(test)]

#[path = "logger_tests_behavior/api_url.rs"]
mod api_url;
#[path = "logger_tests_behavior/auth.rs"]
mod auth;
#[path = "logger_tests_behavior/batch.rs"]
mod batch;
#[path = "logger_tests_behavior/callback_logger.rs"]
mod callback_logger;
#[path = "logger_tests_behavior/enqueue.rs"]
mod enqueue;
#[path = "logger_tests_behavior/flush.rs"]
mod flush;
#[path = "logger_tests_behavior/health.rs"]
mod health;
#[path = "logger_tests_behavior/init.rs"]
mod init;
#[path = "logger_tests_behavior/lazy.rs"]
mod lazy;
#[path = "logger_tests_behavior/lifecycle.rs"]
mod lifecycle;
#[path = "logger_tests_behavior/overflow.rs"]
mod overflow;
#[path = "logger_tests_behavior/ping.rs"]
mod ping;
#[path = "logger_tests_behavior/redaction.rs"]
mod redaction;
#[path = "logger_tests_behavior/send.rs"]
mod send;
#[path = "logger_tests_behavior/support.rs"]
mod support;
