use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mlua::{Lua, Value};
use redis::Script;

use super::resp::{
    Command, CommandHandler, Entries, NIL_REPLY, StoredEntry, connection_answer, encode_bulk,
    integer, live_entry, locked,
};

const MISSING_TTL_REPLY: i64 = -2;
const NO_EXPIRY_REPLY: i64 = -1;
const INFO_REPLY: &str = "# Server\r\nredis_version:7.0.0\r\n";

pub(crate) struct LeaseCommands {
    pub(crate) entries: Entries,
    pub(crate) scripts: Mutex<BTreeMap<String, String>>,
}

impl LeaseCommands {
    fn set(&self, command: &Command) -> Option<Vec<u8>> {
        let key = command.args.get(1)?;
        let value = command.args.get(2)?;
        let (nx, ttl) = match command.args.get(3..).unwrap_or_default() {
            [] => (false, None),
            [expiry, seconds] if expiry.eq_ignore_ascii_case("EX") => {
                (false, Some(seconds.parse::<i64>().ok()?))
            }
            [condition, expiry, seconds]
                if condition.eq_ignore_ascii_case("NX") && expiry.eq_ignore_ascii_case("EX") =>
            {
                (true, Some(seconds.parse::<i64>().ok()?))
            }
            _ => return None,
        };
        if nx && live_entry(&self.entries, key).is_some() {
            return Some(NIL_REPLY.to_vec());
        }
        let expires_at = ttl
            .and_then(|seconds| u64::try_from(seconds).ok())
            .and_then(|seconds| Instant::now().checked_add(Duration::from_secs(seconds)));
        let entry = StoredEntry {
            value: value.clone(),
            ttl_seconds: ttl,
            expires_at,
        };
        locked(&self.entries).insert(key.clone(), entry);
        Some(b"+OK\r\n".to_vec())
    }

    fn get(&self, command: &Command) -> Option<Vec<u8>> {
        let key = command.args.get(1)?;
        Some(
            live_entry(&self.entries, key)
                .map_or_else(|| NIL_REPLY.to_vec(), |entry| encode_bulk(&entry.value)),
        )
    }

    fn delete(&self, command: &Command) -> Option<Vec<u8>> {
        let key = command.args.get(1)?;
        let count = deleted(&self.entries, key);
        Some(integer(count))
    }

    fn ttl(&self, command: &Command) -> Option<Vec<u8>> {
        let key = command.args.get(1)?;
        let remaining = remaining_seconds(&self.entries, key);
        Some(integer(remaining))
    }

    fn script(&self, command: &Command) -> Option<Vec<u8>> {
        let subcommand = command.args.get(1)?;
        if !subcommand.eq_ignore_ascii_case("LOAD") {
            return None;
        }
        let source = command.args.get(2)?;
        let sha = Script::new(source.as_str()).get_hash().to_owned();
        locked(&self.scripts).insert(sha.clone(), source.clone());
        Some(encode_bulk(&sha))
    }

    fn evalsha(&self, command: &Command) -> Option<Vec<u8>> {
        let sha = command.args.get(1)?;
        let loaded = locked(&self.scripts).get(sha).cloned();
        let Some(source) = loaded else {
            return Some(b"-NOSCRIPT No matching script. Please use EVAL.\r\n".to_vec());
        };
        let key_count = command.args.get(2)?.parse::<usize>().ok()?;
        let first_argument = key_count.checked_add(3)?;
        let keys = command.args.get(3..first_argument)?.to_vec();
        let arguments = command
            .args
            .get(first_argument..)
            .unwrap_or_default()
            .to_vec();
        Some(run_script(&source, &keys, &arguments, &self.entries))
    }
}

fn deleted(entries: &Entries, key: &str) -> i64 {
    if live_entry(entries, key).is_none() {
        return 0;
    }
    let _removed = locked(entries).remove(key);
    1
}

// Redis answers -2 for a missing key and -1 for one without an expiry; a live lease reports whole seconds rounded up.
fn remaining_seconds(entries: &Entries, key: &str) -> i64 {
    let Some(entry) = live_entry(entries, key) else {
        return MISSING_TTL_REPLY;
    };
    let Some(expires_at) = entry.expires_at else {
        return NO_EXPIRY_REPLY;
    };
    let remaining = expires_at.saturating_duration_since(Instant::now());
    let seconds = remaining.as_millis().div_ceil(1000);
    i64::try_from(seconds).unwrap_or(i64::MAX)
}

// Redis runs the loaded script itself, so the production Lua decides; redis.call reaches the same in-process store.
fn run_script(source: &str, keys: &[String], arguments: &[String], entries: &Entries) -> Vec<u8> {
    match execute(source, keys, arguments, entries) {
        Ok(reply) => reply,
        Err(error) => resp_error(&error.to_string()),
    }
}

fn execute(
    source: &str,
    keys: &[String],
    arguments: &[String],
    entries: &Entries,
) -> Result<Vec<u8>, mlua::Error> {
    // Redis scripts run without the standard libraries; the fixture's scripts get only redis.call, KEYS and ARGV.
    let lua = Lua::new_with(mlua::StdLib::NONE, mlua::LuaOptions::default())?;
    let redis = lua.create_table()?;
    let store = Arc::clone(entries);
    let call = lua.create_function(move |lua, (name, key): (String, String)| {
        redis_call(lua, &store, &name, &key)
    })?;
    redis.set("call", call)?;
    lua.globals().set("redis", redis)?;
    lua.globals()
        .set("KEYS", lua.create_sequence_from(keys.iter().cloned())?)?;
    lua.globals()
        .set("ARGV", lua.create_sequence_from(arguments.iter().cloned())?)?;
    script_reply(lua.load(source).eval::<Value>()?)
}

// The source script calls only get and del; another command is outside the fixture's supported subset.
fn redis_call(lua: &Lua, store: &Entries, name: &str, key: &str) -> Result<Value, mlua::Error> {
    match name.to_ascii_lowercase().as_str() {
        "get" => match live_entry(store, key) {
            Some(entry) => Ok(Value::String(lua.create_string(&entry.value)?)),
            None => Ok(Value::Boolean(false)),
        },
        "del" => Ok(Value::Integer(deleted(store, key))),
        other => Err(mlua::Error::RuntimeError(format!(
            "unknown redis command '{other}'"
        ))),
    }
}

fn script_reply(value: Value) -> Result<Vec<u8>, mlua::Error> {
    match value {
        Value::Integer(count) => Ok(integer(count)),
        Value::Nil | Value::Boolean(false) => Ok(NIL_REPLY.to_vec()),
        Value::Boolean(true) => Ok(integer(1)),
        other => Err(mlua::Error::RuntimeError(format!(
            "the script returned an unsupported value: {other:?}"
        ))),
    }
}

// A RESP error is one line, so the Lua message's line breaks become spaces.
fn resp_error(message: &str) -> Vec<u8> {
    let flattened = message.replace(['\r', '\n'], " ");
    format!("-ERR {flattened}\r\n").into_bytes()
}

impl CommandHandler for LeaseCommands {
    fn answer(&self, command: &Command) -> Option<Vec<u8>> {
        match command.name.as_str() {
            "SET" => self.set(command),
            "GET" => self.get(command),
            "DEL" => self.delete(command),
            "TTL" => self.ttl(command),
            "SCRIPT" => self.script(command),
            "EVALSHA" => self.evalsha(command),
            "INFO" => Some(encode_bulk(INFO_REPLY)),
            _ => connection_answer(command),
        }
    }
}
