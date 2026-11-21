mod fixture;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_auth::auth_team_lookup::access::get_access_object;
use betterlite_auth::auth_team_lookup::alias::get_team_object_by_alias;
use betterlite_auth::auth_team_lookup::error::{Detail, TeamLookupError};
use betterlite_auth::auth_team_lookup::lookup::{
    TeamDatabase, TeamLookup, TeamRequest, TeamSource,
};
use betterlite_auth::auth_team_lookup::team::get_team_object;
use betterlite_auth::auth_user_lookup::cadence::Cadence;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;
use tracing::subscriber::set_default;
use tracing_subscriber::layer::SubscriberExt as _;

use self::fixture::{Board, Levels, Port, Script};

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    Team,
    Alias,
    Access,
}

#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Flag {
    NoDatabase,
    CacheOnly,
    DbOnly,
    Upsert,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Step {
    now: u64,
    fail: Vec<String>,
}

// one lookup run once per step against the same cache, tables and last_db_access_time
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    row: String,
    function: Function,
    target: String,
    flags: Vec<Flag>,
    in_memory_ttl: i32,
    throttled_at: Option<u64>,
    cached_teams: BTreeMap<String, LiteLLMTeamTableCachedObj>,
    cached_groups: BTreeMap<String, AccessGroupTable>,
    teams: Vec<JsonData>,
    groups: Vec<JsonData>,
    permissions: Vec<JsonData>,
    created: Option<JsonData>,
    steps: Vec<Step>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().enable_all().build()?;
        let (script, lookup) = wire(input);
        if let Some(at) = input.throttled_at {
            script
                .board
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .now = Duration::from_secs(at);
            lookup
                .cadence
                .update(&format!("team_id:{}", input.target), None);
        }
        let levels = Levels {
            script: Arc::clone(&script),
        };
        let _logging = set_default(tracing_subscriber::registry().with(levels));
        let mut steps = Vec::with_capacity(input.steps.len());
        for step in &input.steps {
            begin(&script, step);
            let (value, error) = runtime.block_on(outcome(&lookup, input));
            let board = script.board.lock().unwrap_or_else(PoisonError::into_inner);
            steps.push(
                json!({"value": value, "error": error, "events": board.events, "logs": board.logs}),
            );
        }
        Ok(json!({"row": input.row, "steps": steps}))
    }
}

fn begin(script: &Script, step: &Step) {
    let mut board = script.board.lock().unwrap_or_else(PoisonError::into_inner);
    board.now = Duration::from_secs(step.now);
    board.fail.clone_from(&step.fail);
    board.events.clear();
    board.logs.clear();
}

// the case's cache and tables behind every port; LiteLLM's prisma_client.db is the reader, writer_db the writer
fn wire(input: &Input) -> (Arc<Script>, TeamLookup) {
    let board = Board {
        teams: input.cached_teams.clone(),
        groups: input.cached_groups.clone(),
        ..Board::default()
    };
    let script = Arc::new(Script {
        board: Mutex::new(board),
        teams: input.teams.clone(),
        groups: input.groups.clone(),
        permissions: input.permissions.clone(),
        created: input.created.clone(),
    });
    let port = |handle: &'static str| {
        Arc::new(Port {
            script: Arc::clone(&script),
            handle,
        })
    };
    let database = TeamDatabase {
        store: port("reader"),
        permission_reader: port("reader"),
        permission_writer: port("writer"),
    };
    let lookup = TeamLookup {
        cache: port("reader"),
        permission_cache: port("reader"),
        database: (!input.flags.contains(&Flag::NoDatabase)).then_some(database),
        provisioner: Some(port("reader")),
        cadence: Arc::new(Cadence::new(100, port("reader"))),
        in_memory_ttl: input.in_memory_ttl,
    };
    (script, lookup)
}

async fn outcome(lookup: &TeamLookup, input: &Input) -> (Value, Value) {
    let flags = &input.flags;
    let answer = match input.function {
        Function::Team => {
            let request = TeamRequest {
                team_id: &input.target,
                source: source(flags),
                team_id_upsert: flags.contains(&Flag::Upsert),
            };
            get_team_object(lookup, request).await.map(|team| team_view(&team))
        }
        Function::Alias => get_team_object_by_alias(lookup, &input.target)
            .await
            .map(|team| team_view(&team)),
        Function::Access => get_access_object(lookup, &input.target, flags.contains(&Flag::DbOnly))
            .await
            .map(|group| json!({"access_group_id": group.access_group_id, "access_group_name": group.access_group_name})),
    };
    match answer {
        Ok(value) => (value, Value::Null),
        Err(error) => (Value::Null, failure(&error)),
    }
}

// check_db_only wins over check_cache_only, as LiteLLM's `if not check_db_only` reads the cache first
fn source(flags: &[Flag]) -> TeamSource {
    if flags.contains(&Flag::DbOnly) {
        return TeamSource::Writer;
    }
    if flags.contains(&Flag::CacheOnly) {
        TeamSource::CacheOnly
    } else {
        TeamSource::CacheThenDatabase
    }
}

fn team_view(team: &LiteLLMTeamTableCachedObj) -> Value {
    let fields = &team.team.fields;
    json!({
        "team_id": fields.base.team_id,
        "team_alias": fields.base.team_alias,
        "permission": fields.object_permission.is_some(),
        "model_table": fields.litellm_model_table.is_some(),
        "refreshed": team.last_refreshed_at.is_some(),
    })
}

// the exception class, status and detail shape; the message text is not compared
fn failure(error: &TeamLookupError) -> Value {
    let Some((status, detail)) = error.http() else {
        return json!({"kind": "exception", "status": null, "detail": null});
    };
    let kind = if matches!(error, TeamLookupError::TeamNotFound { .. }) {
        "team_not_found"
    } else {
        "http"
    };
    let shape = match detail {
        Detail::Error => "object",
        Detail::Text => "text",
    };
    json!({"kind": kind, "status": status, "detail": shape})
}
