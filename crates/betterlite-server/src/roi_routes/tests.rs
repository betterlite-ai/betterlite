#![cfg(test)]

pub(crate) mod support;

use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_roi::records::{ROISyncPhase, ROISyncStatus};
use betterlite_roi::sample::sample_report;
use betterlite_roi::settings::ROISettingsResponse;

use crate::roi_connections::access::{READ_FORBIDDEN, WRITE_FORBIDDEN};
use crate::roi_connections::gateway_key::MISSING_KEY;
use crate::roi_routes::error::{
    ALREADY_RUNNING, INVALID_IDENTITY, MODE_LITERAL, NOT_READY, RESTART_BLOCKED, RoiRoutesError,
};
use crate::roi_routes::manager::StoreCoordinator;
use crate::roi_routes::ports::SyncStorePort;
use crate::roi_routes::schedule::{next_update, run_scheduled_sync};
use crate::roi_routes::{IDENTITY_MAP_PATH, REPORT_PATH, SETUP_RESET_PATH, SYNC_PATH};
use crate::roi_sync::coordinator::SyncCoordinator;

use self::support::{
    FakeStore, Harness, IdentityReply, ReportReply, SEALED_ESTIMATOR, SEALED_TOKEN, SummaryPeek,
    admin, call, delete, finished_status, fixed_now, get, idle_status, member, minutes_before,
    missing_key_completion, post, put, ready_stored, recorded, report_json, running_status,
    settings_from, stored_json, stored_peek, viewer,
};

include!("tests/route.rs");
include!("tests/sync.rs");
include!("tests/report.rs");
include!("tests/identity.rs");
include!("tests/reset.rs");
include!("tests/schedule.rs");
include!("tests/oneshot.rs");
