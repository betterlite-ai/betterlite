#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Ready, ready};
use std::sync::Mutex;

use betterlite_auth::agent_caller::caller_auth::caller_record;
use betterlite_auth::agent_policy::caller::AgentCaller;
use betterlite_auth::resolved_key::auth::{AuthInput, ParentOtelSpan, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::{JsonData, PermissionList};
use thiserror::Error;

use super::agent::AgentCeilings as _;
use super::agent::agent_capped_servers;
use super::caller::AgentCallerCeilings as _;
use super::end_user::EndUserCeilings as _;
use super::error::{CeilingReadError, CeilingsError};
use super::live::decode;
use super::org::OrgCeilings as _;
use super::reads::{AdmissionMarks, CeilingReads};
use super::user::UserCeilings as _;
use super::{Capped, CeilingServers, CeilingSource, Ceilings};
use crate::auth::object_permission_hydration::error::UnloadableEntitlementError;
use crate::auth::object_permission_hydration::reads::ObjectPermission;
use crate::server_grants::ToolGrants;

include!("tests/support.rs");
include!("tests/org.rs");
include!("tests/user.rs");
include!("tests/agent.rs");
include!("tests/tools.rs");
include!("tests/identity.rs");
