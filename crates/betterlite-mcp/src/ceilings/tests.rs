#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Ready, ready};
use std::sync::Mutex;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::{JsonData, PermissionList};
use thiserror::Error;

use super::agent::agent_capped_servers;
use super::error::{CeilingReadError, CeilingsError};
use super::reads::{AdmissionMarks, CeilingReads};
use super::{Capped, CeilingLevels as _, CeilingServers, CeilingSource, Ceilings};
use crate::auth::object_permission_hydration::error::UnloadableEntitlementError;
use crate::auth::object_permission_hydration::reads::ObjectPermission;
use crate::auth::object_permission_hydration::record::McpAuth;
use crate::server_grants::ToolGrants;

include!("tests/support.rs");
include!("tests/org.rs");
include!("tests/user.rs");
include!("tests/agent.rs");
include!("tests/tools.rs");
