use std::net::SocketAddr;
use std::sync::Arc;

use crate::http_scenarios::error::ScenarioError;
use crate::redis_fixture::resp::{
    Command, CommandHandler, NIL_REPLY, RespServer, connection_answer, encode_bulk, live_entry,
};

pub(super) use crate::redis_fixture::resp::{Entries, StoredEntry, locked};

// The polling profile answers plain and EX SET, records every other command as unsupported, and never expires an entry.
pub(super) struct RedisServer {
    pub(super) addr: SocketAddr,
    pub(super) entries: Entries,
    pub(super) inner: RespServer,
}

impl RedisServer {
    pub(super) fn start() -> Result<Self, ScenarioError> {
        let entries: Entries = Entries::default();
        let handler: Arc<dyn CommandHandler> = Arc::new(PollingCommands {
            entries: Arc::clone(&entries),
        });
        let inner = RespServer::start(handler).map_err(ScenarioError::Io)?;
        let addr = inner.addr;
        Ok(Self {
            addr,
            entries,
            inner,
        })
    }
}

struct PollingCommands {
    entries: Entries,
}

impl CommandHandler for PollingCommands {
    fn answer(&self, command: &Command) -> Option<Vec<u8>> {
        match command.name.as_str() {
            "SET" => set_answer(&self.entries, command),
            "GET" => Some(get_answer(&self.entries, command)),
            _ => connection_answer(command),
        }
    }
}

// only the plain and EX forms are implemented; any other option is reported as unsupported
fn set_answer(entries: &Entries, command: &Command) -> Option<Vec<u8>> {
    let ttl = match command.args.get(3..).unwrap_or_default() {
        [] => None,
        [option, seconds] if option.eq_ignore_ascii_case("EX") => {
            Some(seconds.parse::<i64>().ok()?)
        }
        _ => return None,
    };
    let key = command.args.get(1)?;
    let value = command.args.get(2)?;
    let entry = StoredEntry {
        value: value.clone(),
        ttl_seconds: ttl,
        expires_at: None,
    };
    locked(entries).insert(key.clone(), entry);
    Some(b"+OK\r\n".to_vec())
}

fn get_answer(entries: &Entries, command: &Command) -> Vec<u8> {
    let key = command.args.get(1);
    let stored = key.and_then(|key| live_entry(entries, key));
    stored.map_or_else(|| NIL_REPLY.to_vec(), |entry| encode_bulk(&entry.value))
}
