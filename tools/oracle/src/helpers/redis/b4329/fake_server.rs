use std::collections::BTreeMap;
use std::io;
use std::mem;
use std::net::SocketAddr;
use std::str;
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::io::{AsyncBufRead, AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

type Lists = BTreeMap<String, Vec<Vec<u8>>>;
pub(crate) type LpopLog = Arc<Mutex<Vec<usize>>>;

pub(crate) struct FakeServer {
    pub(crate) addr: SocketAddr,
    lists: Arc<Mutex<Lists>>,
    tasks: JoinSet<()>,
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.tasks.abort_all();
    }
}

impl FakeServer {
    pub(crate) async fn start(initial: Lists, log: LpopLog) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let lists = Arc::new(Mutex::new(initial));
        let shared = Arc::clone(&lists);
        let shared_log = Arc::clone(&log);
        let mut tasks = JoinSet::new();
        tasks.spawn(async move { accept_loop(listener, shared, shared_log).await });
        Ok(Self { addr, lists, tasks })
    }

    pub(crate) fn lists(&self) -> Lists {
        let guard = match self.lists.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.clone()
    }
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[derive(Clone)]
struct Command {
    name: String,
    args: Vec<Vec<u8>>,
}

impl Command {
    fn number(&self, index: usize) -> Option<i64> {
        let raw = self.args.get(index)?;
        let text = str::from_utf8(raw).ok()?;
        text.parse().ok()
    }
}

#[derive(Default)]
struct Session {
    in_multi: bool,
    queued: Vec<Command>,
}

async fn accept_loop(listener: TcpListener, lists: Arc<Mutex<Lists>>, log: LpopLog) {
    let mut connections = JoinSet::new();
    while let Ok((stream, _peer)) = listener.accept().await {
        connections.spawn(serve(stream, Arc::clone(&lists), Arc::clone(&log)));
        while connections.try_join_next().is_some() {}
    }
}

async fn serve(stream: TcpStream, lists: Arc<Mutex<Lists>>, log: LpopLog) {
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut reader = tokio::io::BufReader::new(read_half);
    let mut session = Session::default();
    while let Ok(Some(command)) = read_command(&mut reader).await {
        if command.name == "LPOP" {
            locked(&log).push(command.args.len());
        }
        let replies = {
            let mut guard = locked(&lists);
            reply_for(&mut guard, &mut session, &command)
        };
        for reply in replies {
            if write_half.write_all(&reply).await.is_err() {
                return;
            }
        }
        if write_half.flush().await.is_err() {
            return;
        }
    }
}

async fn read_command<R>(reader: &mut R) -> io::Result<Option<Command>>
where
    R: AsyncBufRead + Unpin,
{
    let mut header = String::new();
    if reader.read_line(&mut header).await? == 0 {
        return Ok(None);
    }
    let Some(count_text) = header.trim().strip_prefix('*') else {
        return Ok(None);
    };
    let count: usize = count_text
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut args = Vec::with_capacity(count);
    for _ in 0..count {
        let mut length_line = String::new();
        if reader.read_line(&mut length_line).await? == 0 {
            return Ok(None);
        }
        let Some(length_text) = length_line.trim().strip_prefix('$') else {
            return Ok(None);
        };
        let length: usize = length_text
            .parse()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let mut buffer = vec![0_u8; length.saturating_add(2)];
        reader.read_exact(&mut buffer).await?;
        buffer.truncate(length);
        args.push(buffer);
    }
    let Some(raw_name) = args.first() else {
        return Ok(None);
    };
    let command_name = String::from_utf8(raw_name.clone())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(Some(Command {
        name: command_name.to_ascii_uppercase(),
        args,
    }))
}

fn reply_for(lists: &mut Lists, session: &mut Session, command: &Command) -> Vec<Vec<u8>> {
    match command.name.as_str() {
        "MULTI" => {
            session.in_multi = true;
            session.queued.clear();
            vec![b"+OK\r\n".to_vec()]
        }
        "EXEC" => {
            session.in_multi = false;
            let queued = mem::take(&mut session.queued);
            let mut out = format!("*{}\r\n", queued.len()).into_bytes();
            for pending in &queued {
                out.extend(apply(lists, pending));
            }
            vec![out]
        }
        _ if session.in_multi => {
            session.queued.push(command.clone());
            vec![b"+QUEUED\r\n".to_vec()]
        }
        _ => vec![apply(lists, command)],
    }
}

fn apply(lists: &mut Lists, command: &Command) -> Vec<u8> {
    let name = command.name.as_str();
    let Some(key) = command
        .args
        .get(1)
        .and_then(|argument| str::from_utf8(argument).ok())
    else {
        return b"+OK\r\n".to_vec();
    };
    if name == "RPUSH" {
        let values: Vec<Vec<u8>> = command.args.iter().skip(2).cloned().collect();
        if values.is_empty() {
            return error_reply("ERR wrong number of arguments");
        }
        let rows = lists.entry(key.to_owned()).or_default();
        rows.extend(values);
        return i64::try_from(rows.len()).map_or_else(
            |_error| error_reply("ERR the list is too long"),
            |length| format!(":{length}\r\n").into_bytes(),
        );
    }
    if name == "LTRIM" {
        let (Some(start), Some(stop)) = (command.number(2), command.number(3)) else {
            return error_reply("ERR wrong number of arguments");
        };
        if let Some(rows) = lists.get_mut(key) {
            trim_rows(rows, start, stop);
        }
        return b"+OK\r\n".to_vec();
    }
    if name == "LPOP" {
        return pop_reply(lists, key, command);
    }
    b"+OK\r\n".to_vec()
}

fn pop_reply(lists: &mut Lists, key: &str, command: &Command) -> Vec<u8> {
    let count = match command.args.get(2) {
        Some(_raw) => match command.number(2) {
            Some(value) => Some(value),
            None => return error_reply("ERR value is not an integer or out of range"),
        },
        None => None,
    };
    if matches!(count, Some(value) if value < 0) {
        return error_reply("ERR value is out of range, must be positive");
    }
    let Some(rows) = lists.get_mut(key).filter(|rows| !rows.is_empty()) else {
        return b"$-1\r\n".to_vec();
    };
    match count {
        None => bulk_reply(&rows.remove(0)),
        Some(limit) => {
            let take = usize::try_from(limit).map_or(rows.len(), |value| value.min(rows.len()));
            let popped: Vec<Vec<u8>> = rows.drain(..take).collect();
            let mut out = format!("*{}\r\n", popped.len()).into_bytes();
            for value in &popped {
                out.extend(bulk_reply(value));
            }
            out
        }
    }
}

fn trim_rows(rows: &mut Vec<Vec<u8>>, start: i64, stop: i64) {
    let Ok(length) = i64::try_from(rows.len()) else {
        rows.clear();
        return;
    };
    let requested = if start < 0 {
        length.saturating_add(start)
    } else {
        start
    };
    let begin = requested.max(0);
    let requested_end = if stop < 0 {
        length.saturating_add(stop)
    } else {
        stop
    };
    let end = requested_end.min(length.saturating_sub(1));
    if begin > end {
        rows.clear();
        return;
    }
    let (Ok(begin_index), Ok(end_index)) = (usize::try_from(begin), usize::try_from(end)) else {
        rows.clear();
        return;
    };
    rows.truncate(end_index.saturating_add(1));
    rows.drain(..begin_index);
}

fn bulk_reply(bytes: &[u8]) -> Vec<u8> {
    let mut out = format!("${}\r\n", bytes.len()).into_bytes();
    out.extend_from_slice(bytes);
    out.extend_from_slice(b"\r\n");
    out
}

fn error_reply(text: &str) -> Vec<u8> {
    let mut reply = b"-".to_vec();
    reply.extend_from_slice(text.as_bytes());
    reply.extend_from_slice(b"\r\n");
    reply
}
