#![cfg(test)]

use std::sync::Arc;

use rust_decimal::Decimal;

use super::fixtures::{
    FakeLease, FakeScheduler, FakeTransport, FixedClock, clone_store, close, database, datastore,
    insert_usage, lease_events, leased, logger, minutes_ago, settings, settings_from, usage,
};
use crate::finops::cloudzero::schedule::CloudZeroLogger;
use crate::finops::cloudzero::schedule::EXPORT_JOB_NAME;
use crate::finops::cloudzero::schedule::background::init_cloudzero_background_job;

mod registration;

mod no_loggers;

mod job_body;

mod job_failure;
