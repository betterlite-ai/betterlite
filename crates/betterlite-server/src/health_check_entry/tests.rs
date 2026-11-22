#![cfg(test)]

use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use self::support::{
    HEALTHY_HEADERS, Harness, Recorded, RequestedResolution, assert_uuid4, failed, fixed_time,
    healthy, params, resolved, stack_of, state_of,
};
use super::ports::{CatalogMode, CatalogModes as _};
use super::{HealthEntryError, RawRequestRecord};
use crate::health_probe_modes::response::HealthCheckResponse;

mod support;

include!("tests/flow.rs");
include!("tests/mode_lookup.rs");
include!("tests/wildcard.rs");
include!("tests/headers.rs");
include!("tests/errors.rs");
