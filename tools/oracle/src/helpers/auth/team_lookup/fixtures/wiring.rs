use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_auth::auth_team_lookup::lookup::{TeamDatabase, TeamLookup};
use betterlite_auth::auth_user_lookup::cadence::Cadence;

use super::fakes::{Board, Port, Script};
use super::input::{Flag, Input, Step};

pub(crate) fn begin(script: &Script, step: &Step) {
    let mut board = script.board.lock().unwrap_or_else(PoisonError::into_inner);
    board.now = Duration::from_secs(step.now);
    board.fail.clone_from(&step.fail);
    board.events.clear();
    board.logs.clear();
}

// the case's cache and tables behind every port; LiteLLM's prisma_client.db is the reader, writer_db the writer
pub(crate) fn wire(input: &Input) -> (Arc<Script>, TeamLookup) {
    let board = Board {
        events: Vec::new(),
        logs: Vec::new(),
        fail: Vec::new(),
        now: Duration::ZERO,
        teams: input.cached_teams.clone(),
        groups: input.cached_groups.clone(),
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
