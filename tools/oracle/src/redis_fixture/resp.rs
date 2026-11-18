use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, Read, Write as _};
use std::mem;
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const MAX_BULK_BYTES: usize = 16 * 1024 * 1024;
const MAX_HEADER_BYTES: u64 = 64 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) const NIL_REPLY: &[u8] = b"$-1\r\n";

#[derive(Clone)]
pub(crate) struct StoredEntry {
    pub(crate) value: String,
    pub(crate) ttl_seconds: Option<i64>,
    pub(crate) expires_at: Option<Instant>,
}

pub(crate) type Registry = BTreeMap<String, StoredEntry>;
pub(crate) type Entries = Arc<Mutex<Registry>>;

pub(crate) struct Command {
    pub(crate) name: String,
    pub(crate) args: Vec<String>,
}

pub(crate) trait CommandHandler: Send + Sync {
    fn answer(&self, command: &Command) -> Option<Vec<u8>>;
}

struct Shared {
    connections: Arc<Mutex<Vec<TcpStream>>>,
    served: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

// No redis-server binary exists in the gate image, so the oracle's Redis listens in process on a loopback port.
pub(crate) struct RespServer {
    pub(crate) addr: SocketAddr,
    pub(crate) unsupported: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
    shared: Shared,
}

impl RespServer {
    pub(crate) fn start(handler: Arc<dyn CommandHandler>) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let unsupported = Arc::new(Mutex::new(None));
        let connections = Arc::new(Mutex::new(Vec::new()));
        let served = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let slot = Arc::clone(&unsupported);
        let stopping = Arc::clone(&stop);
        let shared = Shared {
            connections: Arc::clone(&connections),
            served: Arc::clone(&served),
        };
        let threads = Shared {
            connections,
            served,
        };
        let accept = thread::Builder::new()
            .spawn(move || accept_loop(&listener, &handler, &slot, &threads, &stopping))?;
        Ok(Self {
            addr,
            unsupported,
            stop,
            accept: Some(accept),
            shared,
        })
    }
}

impl Drop for RespServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _wake = TcpStream::connect(self.addr);
        if let Some(accept) = self.accept.take() {
            let _joined = accept.join();
        }
        close_connections(&self.shared.connections);
        let served = mem::take(&mut *locked(&self.shared.served));
        for handle in served {
            let _joined = handle.join();
        }
    }
}

// A client that is still connected would hold its thread until the read timeout, so every accepted socket is shut down first.
fn close_connections(connections: &Arc<Mutex<Vec<TcpStream>>>) {
    let open = mem::take(&mut *locked(connections));
    for stream in open {
        let _closed = stream.shutdown(Shutdown::Both);
    }
}

fn accept_loop(
    listener: &TcpListener,
    handler: &Arc<dyn CommandHandler>,
    unsupported: &Arc<Mutex<Option<String>>>,
    shared: &Shared,
    stop: &Arc<AtomicBool>,
) {
    while let Ok((stream, _peer)) = listener.accept() {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        if stream.set_read_timeout(Some(READ_TIMEOUT)).is_err() {
            continue;
        }
        if let Ok(tracked) = stream.try_clone() {
            locked(&shared.connections).push(tracked);
        }
        let connection_handler = Arc::clone(handler);
        let slot = Arc::clone(unsupported);
        let Ok(handle) =
            thread::Builder::new().spawn(move || serve(stream, &connection_handler, &slot))
        else {
            continue;
        };
        locked(&shared.served).push(handle);
    }
}

fn serve(
    stream: TcpStream,
    handler: &Arc<dyn CommandHandler>,
    unsupported: &Mutex<Option<String>>,
) {
    let Ok(cloned) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(cloned);
    let mut writer = stream;
    while let Ok(Some(command)) = read_command(&mut reader) {
        let Some(reply) = handler.answer(&command) else {
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

fn read_command<R>(reader: &mut R) -> io::Result<Option<Command>>
where
    R: BufRead,
{
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

fn read_header<R>(reader: &mut R, marker: char) -> io::Result<Option<usize>>
where
    R: BufRead,
{
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

fn read_argument<R>(reader: &mut R, length: usize) -> io::Result<String>
where
    R: BufRead,
{
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

fn record_unsupported(unsupported: &Mutex<Option<String>>, name: &str) {
    let mut slot = locked(unsupported);
    if slot.is_none() {
        *slot = Some(name.to_owned());
    }
}

pub(crate) fn live_entry(entries: &Entries, key: &str) -> Option<StoredEntry> {
    let mut registry = locked(entries);
    let entry = registry.get(key).cloned()?;
    if entry.expires_at.is_some_and(|at| at <= Instant::now()) {
        let _expired = registry.remove(key);
        return None;
    }
    Some(entry)
}

pub(crate) fn encode_bulk(value: &str) -> Vec<u8> {
    let mut reply = format!("${}\r\n", value.len()).into_bytes();
    reply.extend_from_slice(value.as_bytes());
    reply.extend_from_slice(b"\r\n");
    reply
}

pub(crate) fn integer(value: i64) -> Vec<u8> {
    let mut reply = b":".to_vec();
    reply.extend_from_slice(value.to_string().as_bytes());
    reply.extend_from_slice(b"\r\n");
    reply
}

pub(crate) fn connection_answer(command: &Command) -> Option<Vec<u8>> {
    match command.name.as_str() {
        "PING" => Some(b"+PONG\r\n".to_vec()),
        "SELECT" => select_answer(command),
        "QUIT" => Some(b"+OK\r\n".to_vec()),
        "CLIENT" => client_answer(command),
        _ => None,
    }
}

fn select_answer(command: &Command) -> Option<Vec<u8>> {
    let database = command.args.get(1)?;
    if database == "0" {
        return Some(b"+OK\r\n".to_vec());
    }
    None
}

fn client_answer(command: &Command) -> Option<Vec<u8>> {
    let subcommand = command.args.get(1)?;
    if subcommand.eq_ignore_ascii_case("SETINFO") {
        return Some(b"+OK\r\n".to_vec());
    }
    None
}

pub(crate) fn locked<T>(registry: &Mutex<T>) -> MutexGuard<'_, T> {
    match registry.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
