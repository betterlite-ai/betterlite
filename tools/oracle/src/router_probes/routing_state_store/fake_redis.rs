use std::collections::BTreeMap;
use std::io;
use std::mem::take;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::io::{
    AsyncBufRead, AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

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
}

#[derive(Default)]
struct Connection {
    watched: Vec<(String, u64)>,
    multi: bool,
    queued: Vec<Vec<String>>,
}

pub(crate) struct FakeRedis {
    pub(crate) addr: SocketAddr,
    pub(crate) down: Arc<AtomicBool>,
    tasks: JoinSet<()>,
}

impl FakeRedis {
    pub(crate) async fn start() -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let state = Arc::new(State::default());
        let down = Arc::new(AtomicBool::new(false));
        let mut tasks = JoinSet::new();
        tasks.spawn(accept_loop(listener, Arc::clone(&state), Arc::clone(&down)));
        Ok(Self { addr, down, tasks })
    }
}

impl Drop for FakeRedis {
    fn drop(&mut self) {
        self.tasks.abort_all();
    }
}

fn lock(state: &State) -> MutexGuard<'_, Inner> {
    match state.inner.lock() {
        Ok(inner) => inner,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn touch(inner: &mut Inner, key: &str) {
    inner.next_version = inner.next_version.saturating_add(1);
    inner.versions.insert(key.to_owned(), inner.next_version);
}

async fn accept_loop(listener: TcpListener, state: Arc<State>, down: Arc<AtomicBool>) {
    let mut connections = JoinSet::new();
    while let Ok((stream, _peer)) = listener.accept().await {
        if down.load(Ordering::SeqCst) {
            drop(stream);
            continue;
        }
        connections.spawn(serve(stream, Arc::clone(&state), Arc::clone(&down)));
    }
}

async fn serve(stream: TcpStream, state: Arc<State>, down: Arc<AtomicBool>) {
    let (reader, mut writer) = stream.into_split();
    let mut buffered = BufReader::new(reader);
    let mut connection = Connection::default();
    while let Some(args) = read_command(&mut buffered).await {
        if down.load(Ordering::SeqCst) {
            return;
        }
        let reply = dispatch(&state, &mut connection, &args);
        if writer.write_all(&reply).await.is_err() {
            return;
        }
    }
}

async fn read_command<R>(reader: &mut R) -> Option<Vec<String>>
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
    Some(args)
}

fn dispatch(state: &State, connection: &mut Connection, args: &[String]) -> Vec<u8> {
    let name = args
        .first()
        .map(|arg| arg.to_ascii_uppercase())
        .unwrap_or_default();
    if connection.multi && !matches!(name.as_str(), "EXEC") {
        connection.queued.push(args.to_vec());
        return b"+QUEUED\r\n".to_vec();
    }
    match name.as_str() {
        "PING" => b"+PONG\r\n".to_vec(),
        "CLIENT" | "SELECT" | "AUTH" => b"+OK\r\n".to_vec(),
        "WATCH" => watch(state, connection, args),
        "UNWATCH" => {
            connection.watched.clear();
            b"+OK\r\n".to_vec()
        }
        "MULTI" => {
            connection.multi = true;
            connection.queued.clear();
            b"+OK\r\n".to_vec()
        }
        "EXEC" => execute(state, connection),
        _ => run(&mut lock(state), args),
    }
}

fn watch(state: &State, connection: &mut Connection, args: &[String]) -> Vec<u8> {
    let inner = lock(state);
    for key in args.iter().skip(1) {
        let version = inner.versions.get(key).copied().unwrap_or(0);
        connection.watched.push((key.clone(), version));
    }
    drop(inner);
    b"+OK\r\n".to_vec()
}

fn execute(state: &State, connection: &mut Connection) -> Vec<u8> {
    let mut inner = lock(state);
    let changed = connection
        .watched
        .iter()
        .any(|(key, version)| inner.versions.get(key).copied().unwrap_or(0) != *version);
    connection.multi = false;
    connection.watched.clear();
    let queued = take(&mut connection.queued);
    if changed {
        return b"*-1\r\n".to_vec();
    }
    array(queued.iter().map(|args| run(&mut inner, args)).collect())
}

fn run(inner: &mut Inner, args: &[String]) -> Vec<u8> {
    match args.first().map(|arg| arg.to_ascii_uppercase()).as_deref() {
        Some("GET") => bulk(
            inner
                .entries
                .get(arg(args, 1))
                .map(|entry| entry.value.as_slice()),
        ),
        Some("SET") => set(inner, args),
        Some("TTL") => match inner.entries.get(arg(args, 1)) {
            None => b":-2\r\n".to_vec(),
            Some(entry) if entry.ttl < 0 => b":-1\r\n".to_vec(),
            Some(entry) => format!(":{}\r\n", entry.ttl).into_bytes(),
        },
        _ => b"+OK\r\n".to_vec(),
    }
}

fn arg(args: &[String], index: usize) -> &str {
    match args.get(index) {
        Some(arg) => arg.as_str(),
        None => "",
    }
}

fn set(inner: &mut Inner, args: &[String]) -> Vec<u8> {
    let key = arg(args, 1);
    let ttl = if args
        .get(3)
        .is_some_and(|option| option.eq_ignore_ascii_case("EX"))
    {
        args.get(4).and_then(|text| text.parse().ok()).unwrap_or(-1)
    } else {
        -1
    };
    inner.entries.insert(
        key.to_owned(),
        Entry {
            value: arg(args, 2).as_bytes().to_vec(),
            ttl,
        },
    );
    touch(inner, key);
    b"+OK\r\n".to_vec()
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
