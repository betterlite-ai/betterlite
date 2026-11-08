use std::io;
use std::net::{SocketAddr, TcpListener as StdListener};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use tokio::io::{
    AsyncBufRead, AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader, split,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime;
use tokio::task::JoinSet;

const STOP_POLL: Duration = Duration::from_millis(10);

#[derive(Clone, Default)]
pub(crate) struct Replies {
    pub(crate) get: Option<Vec<u8>>,
    pub(crate) mget: Vec<Option<Vec<u8>>>,
}

struct State {
    replies: Replies,
    commands: Mutex<Vec<Vec<String>>>,
}

pub(crate) struct FakeServer {
    pub(crate) addr: SocketAddr,
    state: Arc<State>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<io::Result<()>>>,
}

impl FakeServer {
    pub(crate) fn start(replies: Replies) -> io::Result<Self> {
        let listener = StdListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let state = Arc::new(State {
            replies,
            commands: Mutex::new(Vec::new()),
        });
        let runtime = runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::clone(&state);
        let stopping = Arc::clone(&stop);
        let thread = thread::Builder::new().spawn(move || {
            runtime.block_on(async move {
                let async_listener = TcpListener::from_std(listener)?;
                serve(async_listener, shared, stopping).await
            })
        })?;
        Ok(Self {
            addr,
            state,
            stop,
            thread: Some(thread),
        })
    }

    pub(crate) fn commands(&self) -> Vec<Vec<String>> {
        match self.state.commands.lock() {
            Ok(commands) => commands.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    fn shutdown(&mut self) -> io::Result<()> {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            return thread
                .join()
                .map_err(|_source| io::Error::other("the fake redis thread panicked"))?;
        }
        Ok(())
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        let _shutdown = self.shutdown();
    }
}

async fn serve(listener: TcpListener, state: Arc<State>, stop: Arc<AtomicBool>) -> io::Result<()> {
    let mut connections = JoinSet::new();
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        match tokio::time::timeout(STOP_POLL, listener.accept()).await {
            Ok(Ok((socket, _peer))) => {
                connections.spawn(serve_connection(socket, Arc::clone(&state)));
                while connections.try_join_next().is_some() {}
            }
            Ok(Err(error)) => return Err(error),
            Err(_elapsed) => {}
        }
    }
}

async fn serve_connection(socket: TcpStream, state: Arc<State>) {
    let (read_half, mut write_half) = split(socket);
    let mut reader = BufReader::new(read_half);
    while let Ok(Some(command)) = read_command(&mut reader).await {
        state.record(&command);
        let reply = state.reply(&command);
        if write_half.write_all(&reply).await.is_err() {
            return;
        }
        if write_half.flush().await.is_err() {
            return;
        }
    }
}

struct Command {
    name: String,
    args: Vec<String>,
}

async fn read_command<R>(reader: &mut R) -> io::Result<Option<Command>>
where
    R: AsyncBufRead + Unpin,
{
    let Some(line) = read_line(reader).await? else {
        return Ok(None);
    };
    let Some(count_text) = line.trim().strip_prefix('*') else {
        return Ok(None);
    };
    let count: usize = count_text
        .parse()
        .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
    let mut args = Vec::with_capacity(count);
    for _ in 0..count {
        let Some(header) = read_line(reader).await? else {
            return Ok(None);
        };
        let Some(length_text) = header.trim().strip_prefix('$') else {
            return Ok(None);
        };
        let length: usize = length_text
            .parse()
            .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
        let mut buffer = vec![0_u8; length.saturating_add(2)];
        reader.read_exact(&mut buffer).await?;
        buffer.truncate(length);
        let argument = String::from_utf8(buffer)
            .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
        args.push(argument);
    }
    let Some(name) = args.first() else {
        return Ok(None);
    };
    Ok(Some(Command {
        name: name.to_ascii_uppercase(),
        args,
    }))
}

async fn read_line<R>(reader: &mut R) -> io::Result<Option<String>>
where
    R: AsyncBufRead + Unpin,
{
    let mut line = String::new();
    if reader.read_line(&mut line).await? == 0 {
        return Ok(None);
    }
    Ok(Some(line))
}

impl State {
    fn record(&self, command: &Command) {
        let mut commands = match self.commands.lock() {
            Ok(commands) => commands,
            Err(poisoned) => poisoned.into_inner(),
        };
        commands.push(command.args.clone());
    }

    fn reply(&self, command: &Command) -> Vec<u8> {
        match command.name.as_str() {
            "SADD" => sadd_reply(command),
            "EXPIRE" => b":1\r\n".to_vec(),
            "GET" => encode_bulk(self.replies.get.as_deref()),
            "MGET" => encode_array(&self.replies.mget),
            _ => b"+OK\r\n".to_vec(),
        }
    }
}

fn sadd_reply(command: &Command) -> Vec<u8> {
    if command.args.len() < 3 {
        return b"-ERR wrong number of arguments for 'sadd' command\r\n".to_vec();
    }
    format!(":{}\r\n", command.args.len().saturating_sub(2)).into_bytes()
}

fn encode_bulk(value: Option<&[u8]>) -> Vec<u8> {
    value.map_or_else(
        || b"$-1\r\n".to_vec(),
        |bytes| {
            let mut out = format!("${}\r\n", bytes.len()).into_bytes();
            out.extend_from_slice(bytes);
            out.extend_from_slice(b"\r\n");
            out
        },
    )
}

fn encode_array(values: &[Option<Vec<u8>>]) -> Vec<u8> {
    let mut out = format!("*{}\r\n", values.len()).into_bytes();
    for value in values {
        out.extend(encode_bulk(value.as_deref()));
    }
    out
}
