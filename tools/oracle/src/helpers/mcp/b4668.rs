use std::collections::BTreeMap;
use std::error::Error as _;
use std::future::{Future, ready};

use betterlite_mcp::auth::object_permission_hydration::error::EntitlementError;
use betterlite_mcp::auth::object_permission_hydration::grants::{
    toolset_tool_permissions, toolset_tools_for_server, union_tool_grants,
};
use betterlite_mcp::auth::object_permission_hydration::hydration::load_named_object_permission;
use betterlite_mcp::auth::object_permission_hydration::reads::{
    CachedLink, EndUserEntitlement, ObjectPermission, ObjectPermissionReads, OrgPermissionLink,
};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;
use tokio::runtime::Builder;

const UNLOADABLE: &str = "unloadable_entitlement";
const FAULT: &str = "fault";

// The case scripts the named row read the way the pinned loader sees it: absent, unreadable, or loaded.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReadKind {
    Missing,
    Unreadable,
    Loaded,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    LoadNamed {
        principal: String,
        object_permission_id: String,
        requires_fresh_policy: bool,
        read: ReadKind,
        row: Option<ObjectPermission>,
    },
    Toolset {
        row: Option<ObjectPermission>,
        requires_fresh_policy: bool,
        server_id: Option<String>,
    },
    Union {
        direct: Option<Vec<String>>,
        via_toolsets: Option<Vec<String>>,
    },
}

#[derive(Debug, Error)]
enum Fault {
    #[error("this read is not part of the case")]
    Unscripted,
    #[error("referenced object permission does not exist")]
    MissingRow,
    #[error("the object permission read failed")]
    ReadFailed,
}

// A fresh dispatch process has no database, so the real toolset resolver answers no grants for every id.
struct ScriptedReads {
    read: ReadKind,
    row: Option<ObjectPermission>,
}

impl ObjectPermissionReads for ScriptedReads {
    type Error = Fault;

    fn has_database(&self) -> bool {
        true
    }

    fn management_ttl(&self) -> f64 {
        0.0
    }

    fn cached_link(
        &self,
        _key: &str,
    ) -> impl Future<Output = Result<CachedLink, Self::Error>> + Send {
        ready(Err(Fault::Unscripted))
    }

    fn store_link(
        &self,
        _key: &str,
        _value: &str,
        _ttl: f64,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        ready(Err(Fault::Unscripted))
    }

    // get_object_permission answers None for a missing row or a swallowed fault unless the policy must be fresh
    fn permission(
        &self,
        _object_permission_id: &str,
        check_db_only: bool,
    ) -> impl Future<Output = Result<Option<ObjectPermission>, Self::Error>> + Send {
        let answer = match self.read {
            ReadKind::Loaded => self
                .row
                .as_ref()
                .map_or(Err(Fault::Unscripted), |row| Ok(Some(row.clone()))),
            ReadKind::Missing | ReadKind::Unreadable if !check_db_only => Ok(None),
            ReadKind::Missing => Err(Fault::MissingRow),
            ReadKind::Unreadable => Err(Fault::ReadFailed),
        };
        ready(answer)
    }

    fn team_permission(
        &self,
        _team_id: &str,
        _check_db_only: bool,
    ) -> impl Future<Output = Result<Option<ObjectPermission>, Self::Error>> + Send {
        ready(Err(Fault::Unscripted))
    }

    fn org_object_permission_link(
        &self,
        _org_id: &str,
    ) -> impl Future<Output = Result<OrgPermissionLink, Self::Error>> + Send {
        ready(Err(Fault::Unscripted))
    }

    fn end_user_entitlement(
        &self,
        _end_user_id: &str,
    ) -> impl Future<Output = Result<Option<EndUserEntitlement>, Self::Error>> + Send {
        ready(Err(Fault::Unscripted))
    }

    fn user_object_permission_link(
        &self,
        _user_id: &str,
        _check_db_only: bool,
    ) -> impl Future<Output = Result<Option<String>, Self::Error>> + Send {
        ready(Err(Fault::Unscripted))
    }

    fn agent_object_permission_link(
        &self,
        _agent_id: &str,
    ) -> impl Future<Output = Result<Option<String>, Self::Error>> + Send {
        ready(Err(Fault::Unscripted))
    }

    fn resolve_toolset_grants(
        &self,
        _toolset_ids: &[String],
        _check_db_only: bool,
    ) -> impl Future<Output = Result<BTreeMap<String, Vec<String>>, Self::Error>> + Send {
        ready(Ok(BTreeMap::new()))
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().enable_time().build()?;
        runtime.block_on(answer(input))
    }
}

async fn answer(input: &Input) -> HelperResult {
    Ok(match input {
        Input::LoadNamed {
            principal,
            object_permission_id,
            requires_fresh_policy,
            read,
            row,
        } => {
            let reads = ScriptedReads {
                read: *read,
                row: row.clone(),
            };
            named_answer(
                &reads,
                principal,
                object_permission_id,
                *requires_fresh_policy,
            )
            .await
        }
        Input::Toolset {
            row,
            requires_fresh_policy,
            server_id,
        } => toolset_answer(row.as_ref(), *requires_fresh_policy, server_id.as_deref()).await,
        Input::Union {
            direct,
            via_toolsets,
        } => json!({
            "value": union_tool_grants(direct.as_deref(), via_toolsets.as_deref()),
            "raised": None::<&str>,
            "caused": false,
        }),
    })
}

async fn named_answer(
    reads: &ScriptedReads,
    principal: &str,
    object_permission_id: &str,
    requires_fresh_policy: bool,
) -> Value {
    answered(
        load_named_object_permission(
            reads,
            principal,
            object_permission_id,
            requires_fresh_policy,
        )
        .await
        .map(|loaded| row_view(&loaded))
        .map_err(EntitlementError::from),
    )
}

async fn toolset_answer(
    row: Option<&ObjectPermission>,
    requires_fresh_policy: bool,
    server_id: Option<&str>,
) -> Value {
    let reads = ScriptedReads {
        read: ReadKind::Missing,
        row: None,
    };
    let answer = match server_id {
        Some(server) => toolset_tools_for_server(&reads, row, server, requires_fresh_policy)
            .await
            .map(|tools| json!(tools)),
        None => toolset_tool_permissions(&reads, row, requires_fresh_policy)
            .await
            .map(|grants| json!(grants)),
    };
    answered(answer)
}

fn answered(answer: Result<Value, EntitlementError<Fault>>) -> Value {
    let (value, raised, caused) = match answer {
        Ok(found) => (found, None, false),
        Err(EntitlementError::Read { .. }) => (Value::Null, Some(FAULT), false),
        Err(EntitlementError::Unloadable(inner)) => {
            (Value::Null, Some(UNLOADABLE), inner.source().is_some())
        }
    };
    json!({"value": value, "raised": raised, "caused": caused})
}

fn row_view(row: &ObjectPermission) -> Value {
    json!({
        "object_permission_id": &row.object_permission_id,
        "mcp_toolsets": &row.mcp_toolsets,
    })
}
