#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::future::{Future, ready};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use axum::routing::get as route_get;
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey, UserIdentity,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, Role};
use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::auxiliary_repository::repository::{
    PrismaClient, TableBinding as _, TableError,
};
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_pyhost::json_values::{JsonObject, JsonValue};
use betterlite_redis::config_sync::publish::{PublishChange, PublishFuture};
use chrono::{DateTime, NaiveDateTime, Utc};
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tower::ServiceExt as _;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::auth::auth_identity::AuthContext;
use crate::auth::auth_key_lookup::tests::master_caller;
use crate::guardrails::admin_reads::state::{TeamIdsFuture, UserTeams};

use super::error::SubmissionError;
use super::json_field::parse_json_field;
use super::listing::item;
use super::models::{GuardrailSubmissionItem, decode_register_body};
use super::postgres_rows::{PostgresSubmissionRows, SubmissionRowsBinding};
use super::route::routes;
use super::rows::{
    ColumnJson, NewSubmission, SubmissionRow, SubmissionRows, SubmissionRowsError, TeamFilter,
};
use super::teams::{Caller, user_team_ids};
use super::{Clock, SubmissionsState, SystemClock};

include!("tests/harness.rs");
include!("tests/item.rs");
include!("tests/body.rs");
include!("tests/params.rs");
include!("tests/register.rs");
include!("tests/listing.rs");
include!("tests/reads.rs");
include!("tests/storage.rs");

pub(super) const REGISTER_PARAMS: &str = r#"{"guardrail":"generic_guardrail_api","mode":"pre_call","api_base":"https://guard.example.com/check"}"#;

#[rstest]
fn replays_the_guardrail_register_corpus(
    #[files("fuzz/corpus/guardrail_register/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    drop(decode_register_body(&bytes));
}
