use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::str;
use std::sync::{Arc, Mutex, PoisonError};

use tokio::io::{
    AsyncBufRead, AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader,
};
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::{AbortHandle, JoinSet};

type Reply = mpsc::Sender<Vec<u8>>;

struct State {
    subscribers: Mutex<BTreeMap<String, Vec<Reply>>>,
    connections: Mutex<Vec<AbortHandle>>,
}

impl State {
    fn register_subscriber(&self, channel: &str, sender: &Reply) {
        let mut subscribers = self
            .subscribers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        subscribers
            .entry(channel.to_owned())
            .or_default()
            .push(sender.clone());
    }

    fn register_connection(&self, handle: AbortHandle) {
        let mut connections = self
            .connections
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        connections.push(handle);
    }

    fn deliver(&self, channel: &str, frame: &[u8]) -> usize {
        let mut subscribers = self
            .subscribers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let Some(waiting) = subscribers.get_mut(channel) else {
            return 0;
        };
        let before = waiting.len();
        waiting.retain(|sender| sender.try_send(frame.to_vec()).is_ok());
        let delivered = before.saturating_sub(waiting.len());
        drop(subscribers);
        delivered
    }
}

pub(crate) struct PushServer {
    addr: SocketAddr,
    state: Arc<State>,
    tasks: JoinSet<()>,
}

impl Drop for PushServer {
    fn drop(&mut self) {
        self.tasks.abort_all();
    }
}

impl PushServer {
    pub(crate) async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("an ephemeral listener");
        let addr = listener.local_addr().expect("the listener address");
        let state = Arc::new(State {
            subscribers: Mutex::new(BTreeMap::new()),
            connections: Mutex::new(Vec::new()),
        });
        let mut tasks = JoinSet::new();
        tasks.spawn(accept(listener, Arc::clone(&state)));
        Self { addr, state, tasks }
    }

    #[must_use]
    pub(crate) fn host(&self) -> String {
        self.addr.ip().to_string()
    }

    #[must_use]
    pub(crate) fn port(&self) -> u16 {
        self.addr.port()
    }

    pub(crate) fn close_connections(&self) {
        let handles = self
            .state
            .connections
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for handle in handles.iter() {
            handle.abort();
        }
    }
}

async fn accept(listener: TcpListener, state: Arc<State>) {
    let mut connections = JoinSet::new();
    while let Ok((socket, _peer)) = listener.accept().await {
        let handle = connections.spawn(serve(socket, Arc::clone(&state)));
        state.register_connection(handle);
        while connections.try_join_next().is_some() {}
    }
}

async fn serve(socket: TcpStream, state: Arc<State>) {
    let (read_half, write_half) = socket.into_split();
    let mut reader = BufReader::new(read_half);
    let (sender, receiver) = mpsc::channel::<Vec<u8>>(32);
    let mut writer = JoinSet::new();
    writer.spawn(write_loop(write_half, receiver));
    while let Some(args) = read_command(&mut reader).await {
        if !respond(&state, &args, &sender).await {
            break;
        }
    }
    writer.abort_all();
}

async fn write_loop(mut write_half: OwnedWriteHalf, mut frames: mpsc::Receiver<Vec<u8>>) {
    while let Some(frame) = frames.recv().await {
        if write_half.write_all(&frame).await.is_err() {
            return;
        }
    }
}

async fn respond(state: &State, args: &[String], sender: &Reply) -> bool {
    match args.first().map(String::as_str) {
        Some("SUBSCRIBE") => subscribe(state, args, sender).await,
        Some("PUBLISH") => publish_command(state, args, sender).await,
        Some("QUIT") => false,
        _ => send(sender, b"+OK\r\n".to_vec()).await,
    }
}

async fn subscribe(state: &State, args: &[String], sender: &Reply) -> bool {
    let mut frame = Vec::new();
    for channel in args.iter().skip(1) {
        state.register_subscriber(channel, sender);
        frame.extend(confirmation(channel));
    }
    send(sender, frame).await
}

async fn publish_command(state: &State, args: &[String], sender: &Reply) -> bool {
    let (Some(channel), Some(payload)) = (args.get(1), args.get(2)) else {
        return false;
    };
    let delivered = state.deliver(channel, &message(channel, payload));
    send(sender, format!(":{delivered}\r\n").into_bytes()).await
}

async fn send(sender: &Reply, frame: Vec<u8>) -> bool {
    sender.send(frame).await.is_ok()
}

async fn read_command<R>(reader: &mut R) -> Option<Vec<String>>
where
    R: AsyncBufRead + Unpin,
{
    let count = read_count(reader, b'*').await?;
    let mut args = Vec::with_capacity(count);
    for _ in 0..count {
        let length = read_count(reader, b'$').await?;
        let mut buffer = vec![0_u8; length.saturating_add(2)];
        reader.read_exact(&mut buffer).await.ok()?;
        buffer.truncate(length);
        args.push(String::from_utf8(buffer).ok()?);
    }
    Some(args)
}

async fn read_count<R>(reader: &mut R, prefix: u8) -> Option<usize>
where
    R: AsyncBufRead + Unpin,
{
    let mut line = Vec::new();
    reader.read_until(b'\n', &mut line).await.ok()?;
    let text = str::from_utf8(&line).ok()?.trim();
    text.strip_prefix(char::from(prefix))?.parse().ok()
}

fn bulk(bytes: &[u8]) -> Vec<u8> {
    let mut out = format!("${}\r\n", bytes.len()).into_bytes();
    out.extend(bytes);
    out.extend(b"\r\n");
    out
}

fn confirmation(channel: &str) -> Vec<u8> {
    let mut out = b"*3\r\n".to_vec();
    out.extend(bulk(b"subscribe"));
    out.extend(bulk(channel.as_bytes()));
    out.extend(b":1\r\n");
    out
}

fn message(channel: &str, payload: &str) -> Vec<u8> {
    let mut out = b"*3\r\n".to_vec();
    out.extend(bulk(b"message"));
    out.extend(bulk(channel.as_bytes()));
    out.extend(bulk(payload.as_bytes()));
    out
}
