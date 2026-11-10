use std::collections::BTreeMap;
use std::io;
use std::mem::take;
use std::net::SocketAddr;
use std::str::FromStr as _;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rust_decimal::Decimal;
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

use crate::routing_value_store::RouteValue;

struct Entry {
    value: Vec<u8>,
    ttl: i64,
}

#[derive(Default)]
struct Inner {
    entries: BTreeMap<String, Entry>,
    versions: BTreeMap<String, u64>,
    next_version: u64,
}

#[derive(Default)]
struct State {
    inner: Mutex<Inner>,
    down: AtomicBool,
    reject_auth: AtomicBool,
    rival_writes: AtomicU64,
    handshakes: Mutex<Vec<Vec<String>>>,
}

struct Command {
    args: Vec<String>,
}

#[derive(Default)]
struct ConnectionState {
    watched: Vec<(String, u64)>,
    multi: bool,
    queued: Vec<Command>,
}

pub struct FakeRedis {
    addr: SocketAddr,
    state: Arc<State>,
    _tasks: JoinSet<()>,
}

impl FakeRedis {
    pub async fn try_start() -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let state = Arc::new(State::default());
        let mut tasks = JoinSet::new();
        tasks.spawn(accept_loop(listener, Arc::clone(&state)));
        Ok(Self {
            addr,
            state,
            _tasks: tasks,
        })
    }

    #[cfg(test)]
    pub(crate) async fn start() -> Self {
        Self::try_start().await.expect("the fake redis starts")
    }

    #[must_use]
    pub const fn port(&self) -> u16 {
        self.addr.port()
    }

    pub fn set_down(&self, down: bool) {
        self.state.down.store(down, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn reject_auth(&self, reject: bool) {
        self.state.reject_auth.store(reject, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn rival_writes_before_exec(&self, times: u64) {
        self.state.rival_writes.store(times, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn handshakes(&self) -> Vec<Vec<String>> {
        self.state
            .handshakes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    #[cfg(test)]
    pub(crate) fn raw(&self, key: &str) -> Option<(Vec<u8>, i64)> {
        lock(&self.state)
            .entries
            .get(key)
            .map(|entry| (entry.value.clone(), entry.ttl))
    }

    #[cfg(test)]
    pub(crate) fn seed(&self, key: &str, value: &[u8], ttl: i64) {
        let mut inner = lock(&self.state);
        inner.entries.insert(
            key.to_owned(),
            Entry {
                value: value.to_vec(),
                ttl,
            },
        );
        touch(&mut inner, key);
        drop(inner);
    }
}

fn lock(state: &State) -> MutexGuard<'_, Inner> {
    state.inner.lock().unwrap_or_else(PoisonError::into_inner)
}

fn touch(inner: &mut Inner, key: &str) {
    inner.next_version = inner.next_version.saturating_add(1);
    inner.versions.insert(key.to_owned(), inner.next_version);
}

async fn accept_loop(listener: TcpListener, state: Arc<State>) {
    let mut connections = JoinSet::new();
    while let Ok((stream, _peer)) = listener.accept().await {
        if state.down.load(Ordering::SeqCst) {
            drop(stream);
            continue;
        }
        connections.spawn(serve(stream, Arc::clone(&state)));
    }
}

async fn serve(stream: TcpStream, state: Arc<State>) {
    let (reader, mut writer) = stream.into_split();
    let mut buffered = BufReader::new(reader);
    let mut connection = ConnectionState::default();
    while let Some(command) = read_command(&mut buffered).await {
        if state.down.load(Ordering::SeqCst) {
            return;
        }
        let reply = dispatch(&state, &mut connection, command);
        if writer.write_all(&reply).await.is_err() {
            return;
        }
    }
}

async fn read_command<R>(reader: &mut R) -> Option<Command>
where
    R: AsyncBufRead + Unpin,
{
    let mut line = String::new();
    if reader.read_line(&mut line).await.ok()? == 0 {
        return None;
    }
    let count: usize = line.trim().strip_prefix('*')?.parse().ok()?;
    let mut args = Vec::with_capacity(count);
    for _ in 0..count {
        let mut header = String::new();
        if reader.read_line(&mut header).await.ok()? == 0 {
            return None;
        }
        let length: usize = header.trim().strip_prefix('$')?.parse().ok()?;
        let mut buffer = vec![0_u8; length.saturating_add(2)];
        reader.read_exact(&mut buffer).await.ok()?;
        buffer.truncate(length);
        args.push(String::from_utf8(buffer).ok()?);
    }
    Some(Command { args })
}

fn dispatch(state: &State, connection: &mut ConnectionState, command: Command) -> Vec<u8> {
    let name = command
        .args
        .first()
        .map(|arg| arg.to_ascii_uppercase())
        .unwrap_or_default();
    if connection.multi && name != "EXEC" {
        connection.queued.push(command);
        return simple("QUEUED");
    }
    match name.as_str() {
        "PING" => simple("PONG"),
        "SELECT" | "AUTH" => handshake(state, &command),
        "WATCH" => watch(state, connection, &command),
        "UNWATCH" => {
            connection.watched.clear();
            simple("OK")
        }
        "MULTI" => {
            connection.multi = true;
            connection.queued.clear();
            simple("OK")
        }
        "EXEC" => execute(state, connection),
        _ => {
            let mut inner = lock(state);
            run(&mut inner, &command)
        }
    }
}

fn handshake(state: &State, command: &Command) -> Vec<u8> {
    state
        .handshakes
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(command.args.clone());
    if state.reject_auth.load(Ordering::SeqCst) && arg(command, 0).eq_ignore_ascii_case("AUTH") {
        return error("WRONGPASS invalid username-password pair or user is disabled.");
    }
    simple("OK")
}

fn rival_write(inner: &mut Inner, key: &str) {
    let next = match get(inner, key).map(|bytes| serde_json::from_slice::<RouteValue>(&bytes)) {
        None => 1,
        Some(Ok(RouteValue::Counter(count))) => count.saturating_add(1),
        Some(_) => return touch(inner, key),
    };
    let ttl = inner.entries.get(key).map_or(-1, |entry| entry.ttl);
    let bytes = serde_json::to_vec(&RouteValue::Counter(next)).unwrap_or_default();
    put(inner, key, bytes, ttl);
}

fn watch(state: &State, connection: &mut ConnectionState, command: &Command) -> Vec<u8> {
    let inner = lock(state);
    for key in command.args.iter().skip(1) {
        let version = inner.versions.get(key).copied().unwrap_or(0);
        connection.watched.push((key.clone(), version));
    }
    drop(inner);
    simple("OK")
}

fn execute(state: &State, connection: &mut ConnectionState) -> Vec<u8> {
    if !connection.multi {
        return error("ERR EXEC without MULTI");
    }
    let mut inner = lock(state);
    if state
        .rival_writes
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
            left.checked_sub(1)
        })
        .is_ok()
    {
        for (key, _version) in &connection.watched {
            rival_write(&mut inner, key);
        }
    }
    let changed = connection
        .watched
        .iter()
        .any(|(key, version)| inner.versions.get(key).copied().unwrap_or(0) != *version);
    connection.multi = false;
    connection.watched.clear();
    let queued = take(&mut connection.queued);
    if changed {
        return nil_array();
    }
    let mut replies = Vec::with_capacity(queued.len());
    for command in &queued {
        replies.push(run(&mut inner, command));
    }
    array(replies)
}

fn run(inner: &mut Inner, command: &Command) -> Vec<u8> {
    let name = command
        .args
        .first()
        .map(|arg| arg.to_ascii_uppercase())
        .unwrap_or_default();
    match name.as_str() {
        "GET" => bulk(get(inner, arg(command, 1)).as_deref()),
        "SET" => set(inner, command),
        "MGET" => {
            let replies = command
                .args
                .iter()
                .skip(1)
                .map(|key| bulk(get(inner, key).as_deref()))
                .collect();
            array(replies)
        }
        "DEL" => delete(inner, command),
        "TTL" => integer(
            inner
                .entries
                .get(arg(command, 1))
                .map_or(-2, |entry| entry.ttl),
        ),
        "EXPIRE" => integer(expire(inner, command)),
        "INCRBYFLOAT" => increment(inner, command),
        _ => simple("OK"),
    }
}

fn arg(command: &Command, index: usize) -> &str {
    command.args.get(index).map_or("", String::as_str)
}

fn get(inner: &Inner, key: &str) -> Option<Vec<u8>> {
    inner.entries.get(key).map(|entry| entry.value.clone())
}

fn set(inner: &mut Inner, command: &Command) -> Vec<u8> {
    let options = set_options(command);
    let key = arg(command, 1);
    if options.ttl.is_some_and(|ttl| ttl <= 0) {
        return error("ERR invalid expire time in 'set' command");
    }
    if options.only_missing && inner.entries.contains_key(key) {
        return bulk(None);
    }
    let ttl = if options.keep_ttl {
        inner.entries.get(key).map_or(-1, |entry| entry.ttl)
    } else {
        options.ttl.unwrap_or(-1)
    };
    put(inner, key, arg(command, 2).as_bytes().to_vec(), ttl);
    simple("OK")
}

struct SetOptions {
    ttl: Option<i64>,
    only_missing: bool,
    keep_ttl: bool,
}

fn set_options(command: &Command) -> SetOptions {
    let mut options = SetOptions {
        ttl: None,
        only_missing: false,
        keep_ttl: false,
    };
    let mut index = 3;
    while let Some(option) = command.args.get(index) {
        match option.to_ascii_uppercase().as_str() {
            "EX" => {
                options.ttl = Some(
                    command
                        .args
                        .get(index.saturating_add(1))
                        .and_then(|text| text.parse().ok())
                        .unwrap_or(0),
                );
                index = index.saturating_add(2);
            }
            "NX" => {
                options.only_missing = true;
                index = index.saturating_add(1);
            }
            "KEEPTTL" => {
                options.keep_ttl = true;
                index = index.saturating_add(1);
            }
            _ => index = index.saturating_add(1),
        }
    }
    options
}

fn put(inner: &mut Inner, key: &str, value: Vec<u8>, ttl: i64) {
    inner.entries.insert(key.to_owned(), Entry { value, ttl });
    touch(inner, key);
}

fn delete(inner: &mut Inner, command: &Command) -> Vec<u8> {
    let mut deleted = 0_i64;
    for key in command.args.iter().skip(1) {
        if inner.entries.remove(key).is_some() {
            deleted = deleted.saturating_add(1);
            touch(inner, key);
        }
    }
    integer(deleted)
}

fn expire(inner: &mut Inner, command: &Command) -> i64 {
    let key = arg(command, 1);
    let seconds: i64 = arg(command, 2).parse().unwrap_or(0);
    let Some(entry) = inner.entries.get_mut(key) else {
        return 0;
    };
    entry.ttl = seconds;
    touch(inner, key);
    1
}

fn increment(inner: &mut Inner, command: &Command) -> Vec<u8> {
    let key = arg(command, 1);
    let amount = Decimal::from_str(arg(command, 2)).unwrap_or_default();
    let updated = decimal(inner, key).checked_add(amount).unwrap_or_default();
    let ttl = inner.entries.get(key).map_or(-1, |entry| entry.ttl);
    let text = updated.normalize().to_string();
    put(inner, key, text.clone().into_bytes(), ttl);
    bulk(Some(text.as_bytes()))
}

fn decimal(inner: &Inner, key: &str) -> Decimal {
    get(inner, key)
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|text| Decimal::from_str(&text).ok())
        .unwrap_or_default()
}

fn simple(text: &str) -> Vec<u8> {
    format!("+{text}\r\n").into_bytes()
}

fn error(text: &str) -> Vec<u8> {
    format!("-{text}\r\n").into_bytes()
}

fn integer(value: i64) -> Vec<u8> {
    format!(":{value}\r\n").into_bytes()
}

fn nil_array() -> Vec<u8> {
    b"*-1\r\n".to_vec()
}

fn bulk(bytes: Option<&[u8]>) -> Vec<u8> {
    bytes.map_or_else(
        || b"$-1\r\n".to_vec(),
        |value| {
            let mut reply = format!("${}\r\n", value.len()).into_bytes();
            reply.extend_from_slice(value);
            reply.extend_from_slice(b"\r\n");
            reply
        },
    )
}

fn array(items: Vec<Vec<u8>>) -> Vec<u8> {
    let mut reply = format!("*{}\r\n", items.len()).into_bytes();
    for item in items {
        reply.extend_from_slice(&item);
    }
    reply
}
