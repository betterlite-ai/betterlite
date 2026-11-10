use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, Read, Write as _};
use std::mem;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use super::polling::RedisPolling;
use crate::http_scenarios::error::ScenarioError;

const MAX_BULK_BYTES: usize = 16 * 1024 * 1024;
const MAX_HEADER_BYTES: u64 = 64 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub(super) struct StoredEntry {
    pub(super) value: String,
    pub(super) ttl_seconds: Option<i64>,
}

pub(super) type Registry = BTreeMap<String, StoredEntry>;
pub(super) type Entries = Arc<Mutex<Registry>>;

// B1139 provisions the Redis the two peers seed; no redis-server binary exists in the gate image.
pub(super) struct RedisServer {
    pub(super) addr: SocketAddr,
    entries: Entries,
    unsupported: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
    served: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl RedisServer {
    pub(super) fn start() -> Result<Self, ScenarioError> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(ScenarioError::Io)?;
        let addr = listener.local_addr().map_err(ScenarioError::Io)?;
        let entries: Entries = Arc::new(Mutex::new(Registry::new()));
        let unsupported = Arc::new(Mutex::new(None));
        let served = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::clone(&entries);
        let slot = Arc::clone(&unsupported);
        let threads = Arc::clone(&served);
        let stopping = Arc::clone(&stop);
        let accept = thread::Builder::new()
            .spawn(move || accept_loop(&listener, &shared, &slot, &threads, &stopping))
            .map_err(ScenarioError::Io)?;
        Ok(Self {
            addr,
            entries,
            unsupported,
            stop,
            accept: Some(accept),
            served,
        })
    }

    pub(super) fn url(&self) -> String {
        format!("redis://{}/0", self.addr)
    }

    pub(super) fn unsupported(&self) -> Option<String> {
        locked(&self.unsupported).clone()
    }

    pub(super) fn polling(&self) -> RedisPolling {
        RedisPolling {
            addr: self.addr,
            entries: Arc::clone(&self.entries),
        }
    }
}

impl Drop for RedisServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _wake = TcpStream::connect(self.addr);
        if let Some(accept) = self.accept.take() {
            let _joined = accept.join();
        }
        let served = mem::take(&mut *locked(&self.served));
        for handle in served {
            let _joined = handle.join();
        }
    }
}

fn accept_loop(
    listener: &TcpListener,
    entries: &Entries,
    unsupported: &Arc<Mutex<Option<String>>>,
    served: &Arc<Mutex<Vec<JoinHandle<()>>>>,
    stop: &Arc<AtomicBool>,
) {
    while let Ok((stream, _peer)) = listener.accept() {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        if stream.set_read_timeout(Some(READ_TIMEOUT)).is_err() {
            continue;
        }
        let shared = Arc::clone(entries);
        let slot = Arc::clone(unsupported);
        let Ok(handle) = thread::Builder::new().spawn(move || serve(stream, &shared, &slot)) else {
            continue;
        };
        locked(served).push(handle);
    }
}

fn serve(stream: TcpStream, entries: &Entries, unsupported: &Mutex<Option<String>>) {
    let Ok(cloned) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(cloned);
    let mut writer = stream;
    while let Ok(Some(command)) = read_command(&mut reader) {
        let Some(reply) = answer(entries, &command) else {
            record_unsupported(unsupported, &command.name);
            let error = format!("-ERR unknown command '{}'\r\n", command.name);
            if writer.write_all(error.as_bytes()).is_err() {
                return;
            }
            continue;
        };
        if writer.write_all(&reply).is_err() {
            return;
        }
    }
}

struct Command {
    name: String,
    args: Vec<String>,
}

fn read_command(reader: &mut impl BufRead) -> io::Result<Option<Command>> {
    let Some(count) = read_header(reader, '*')? else {
        return Ok(None);
    };
    let mut args = Vec::new();
    for _ in 0..count {
        let Some(length) = read_header(reader, '$')? else {
            return Ok(None);
        };
        args.push(read_argument(reader, length)?);
    }
    let Some(name) = args.first() else {
        return Ok(None);
    };
    Ok(Some(Command {
        name: name.to_ascii_uppercase(),
        args,
    }))
}

fn read_header(reader: &mut impl BufRead, marker: char) -> io::Result<Option<usize>> {
    let mut line = String::new();
    let read = {
        let mut limited = Read::take(&mut *reader, MAX_HEADER_BYTES);
        limited.read_line(&mut line)?
    };
    if read == 0 {
        return Ok(None);
    }
    if !line.ends_with('\n') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "a header line exceeds 64 KiB",
        ));
    }
    line.trim().strip_prefix(marker).map_or(Ok(None), |text| {
        text.parse::<usize>()
            .map(Some)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    })
}

fn read_argument(reader: &mut impl BufRead, length: usize) -> io::Result<String> {
    if length > MAX_BULK_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "a bulk string exceeds 16 MiB",
        ));
    }
    let mut bytes = vec![0_u8; length.saturating_add(2)];
    reader.read_exact(&mut bytes)?;
    bytes.truncate(length);
    String::from_utf8(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn answer(entries: &Entries, command: &Command) -> Option<Vec<u8>> {
    match command.name.as_str() {
        "SET" => set_answer(entries, command),
        "GET" => Some(lookup(entries, command)),
        "PING" => Some(b"+PONG\r\n".to_vec()),
        "SELECT" => select_answer(command),
        "QUIT" => Some(b"+OK\r\n".to_vec()),
        "CLIENT" => client_answer(command),
        _ => None,
    }
}

fn select_answer(command: &Command) -> Option<Vec<u8>> {
    let database = command.args.get(1)?;
    (database == "0").then_some(b"+OK\r\n".to_vec())
}

fn client_answer(command: &Command) -> Option<Vec<u8>> {
    let subcommand = command.args.get(1)?;
    let supported = subcommand.eq_ignore_ascii_case("SETINFO");
    supported.then_some(b"+OK\r\n".to_vec())
}

fn record_unsupported(unsupported: &Mutex<Option<String>>, name: &str) {
    let mut slot = locked(unsupported);
    if slot.is_none() {
        *slot = Some(name.to_owned());
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
    };
    locked(entries).insert(key.clone(), entry);
    Some(b"+OK\r\n".to_vec())
}

fn lookup(entries: &Entries, command: &Command) -> Vec<u8> {
    command
        .args
        .get(1)
        .and_then(|key| locked(entries).get(key).map(|entry| entry.value.clone()))
        .map_or_else(|| b"$-1\r\n".to_vec(), |value| encode_bulk(&value))
}

fn encode_bulk(value: &str) -> Vec<u8> {
    let mut reply = format!("${}\r\n", value.len()).into_bytes();
    reply.extend_from_slice(value.as_bytes());
    reply.extend_from_slice(b"\r\n");
    reply
}

pub(super) fn locked<T>(registry: &Mutex<T>) -> MutexGuard<'_, T> {
    match registry.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
