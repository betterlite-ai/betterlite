#![cfg(test)]

mod support;

use std::collections::BTreeMap;
use std::fs;
use std::time::Duration;

use betterlite_logging::private_json::files::{FileAccess, LocalFiles};
use rstest::rstest;
use tempfile::TempDir;

use self::support::{
    API_KEY, BASE_URL, FakeEnvironment, FileCall, FileFault, GetAnswer, ScriptedFiles, ScriptedGet,
    api_key, body, read_json,
};
use crate::agent_launch::LITELLM_PROXY_API_KEY_ENV;
use crate::pi::limits::fetch_model_limits;
use crate::pi::listing::{fetch_model_ids, fetch_model_listing};
use crate::pi::models_json::{ModelEntry, models_json_path, provider_block, sync_models_json};
use crate::pi::prepare::{PreparePiContext, prepare_pi};
use crate::pi::{
    ListedModel, ListingFailure, ListingViolation, ModelLimits, PI_CONFIG_DIR_ENV, PiSyncError,
};

include!("tests/listing.rs");
include!("tests/limits.rs");
include!("tests/unique_mappings.rs");
include!("tests/models_json.rs");
include!("tests/prepare.rs");
