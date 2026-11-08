use std::collections::BTreeMap;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::{
    AsyncBufRead, AsyncBufReadExt as _, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _,
    BufReader, split,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

pub(crate) type ClusterSlots = Vec<(u16, u16, Vec<(String, u16)>)>;

#[derive(Clone, Debug)]
pub(crate) struct Recorded {
    pub(crate) name: String,
    pub(crate) args: Vec<String>,
}

#[derive(Default)]
struct State {
    subscribers: Mutex<BTreeMap<String, Vec<mpsc::Sender<Vec<u8>>>>>,
    commands: Mutex<Vec<Recorded>>,
    connections: AtomicUsize,
    slots: Mutex<Option<ClusterSlots>>,
}

pub(crate) struct PubSubServer {
    addr: SocketAddr,
    state: Arc<State>,
    tasks: JoinSet<()>,
}

impl Drop for PubSubServer {
    fn drop(&mut self) {
        self.tasks.abort_all();
    }
}

impl PubSubServer {
    pub(crate) async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("an ephemeral listener");
        let addr = listener.local_addr().expect("the listener address");
        let state = Arc::new(State::default());
        let mut tasks = JoinSet::new();
        let shared = Arc::clone(&state);
        tasks.spawn(async move { accept_loop(listener, shared).await });
        Self { addr, state, tasks }
    }

    pub(crate) fn host(&self) -> String {
        self.addr.ip().to_string()
    }

    pub(crate) fn port(&self) -> u16 {
        self.addr.port()
    }

    pub(crate) fn node(&self) -> (String, u16) {
        (self.host(), self.port())
    }

    pub(crate) fn set_cluster_slots(&self, slots: ClusterSlots) {
        let mut stored = self.state.slots.lock().expect("the slot map");
        *stored = Some(slots);
    }

    pub(crate) fn commands_named(&self, name: &str) -> Vec<Recorded> {
        let wanted = name.to_ascii_uppercase();
        self.state
            .commands
            .lock()
            .expect("the command log")
            .iter()
            .filter(|recorded| recorded.name == wanted)
            .cloned()
            .collect()
    }

    pub(crate) fn connections(&self) -> usize {
        self.state.connections.load(Ordering::Relaxed)
    }
}

async fn accept_loop(listener: TcpListener, state: Arc<State>) {
    let mut connections = JoinSet::new();
    loop {
        let Ok((socket, _peer)) = listener.accept().await else {
            return;
        };
        state.connections.fetch_add(1, Ordering::Relaxed);
        let shared = Arc::clone(&state);
        connections.spawn(async move { serve_connection(socket, shared).await });
        while connections.try_join_next().is_some() {}
    }
}

async fn serve_connection(socket: TcpStream, state: Arc<State>) {
    let (read_half, write_half) = split(socket);
    let mut reader = BufReader::new(read_half);
    let (sender, receiver) = mpsc::channel(64);
    let mut tasks = JoinSet::new();
    tasks.spawn(write_loop(write_half, receiver));
    read_loop(&mut reader, &state, &sender).await;
    tasks.abort_all();
}

async fn write_loop<W>(mut write_half: W, mut receiver: mpsc::Receiver<Vec<u8>>)
where
    W: AsyncWrite + Unpin,
{
    while let Some(frame) = receiver.recv().await {
        if write_half.write_all(&frame).await.is_err() {
            return;
        }
    }
}

async fn read_loop<R>(reader: &mut R, state: &Arc<State>, sender: &mpsc::Sender<Vec<u8>>)
where
    R: AsyncBufRead + Unpin,
{
    while let Ok(Some(command)) = read_command(reader).await {
        state.record(&command);
        if dispatch(state, &command, sender).await {
            return;
        }
    }
}

async fn dispatch(state: &State, command: &Command, sender: &mpsc::Sender<Vec<u8>>) -> bool {
    match command.name.as_str() {
        "SUBSCRIBE" => {
            let frame = {
                let mut subscribers = state.subscribers.lock().expect("the subscriber map");
                let mut frame = Vec::new();
                for channel in command.args.iter().skip(1) {
                    subscribers
                        .entry(channel.clone())
                        .or_default()
                        .push(sender.clone());
                    frame.extend(confirmation(channel));
                }
                drop(subscribers);
                frame
            };
            sender.send(frame).await.is_err()
        }
        "PUBLISH" => {
            let (Some(channel), Some(payload)) = (command.args.get(1), command.args.get(2)) else {
                return false;
            };
            let delivered = publish(state, channel, payload);
            let reply = format!(":{delivered}\r\n").into_bytes();
            sender.send(reply).await.is_err()
        }
        "QUIT" => true,
        _ => sender.send(reply(state, command)).await.is_err(),
    }
}

fn publish(state: &State, channel: &str, payload: &str) -> i64 {
    let mut subscribers = state.subscribers.lock().expect("the subscriber map");
    let mut delivered = 0_i64;
    if let Some(waiting) = subscribers.get_mut(channel) {
        let frame = message(channel, payload);
        waiting.retain(|subscriber| {
            if subscriber.try_send(frame.clone()).is_err() {
                return false;
            }
            delivered = delivered.saturating_add(1);
            true
        });
    }
    drop(subscribers);
    delivered
}

fn reply(state: &State, command: &Command) -> Vec<u8> {
    match command.name.as_str() {
        "PING" => b"+PONG\r\n".to_vec(),
        "CLUSTER" => cluster_reply(state, command),
        _ => b"+OK\r\n".to_vec(),
    }
}

fn cluster_reply(state: &State, command: &Command) -> Vec<u8> {
    let sub = command.args.get(1).map(String::as_str);
    if sub != Some("SLOTS") {
        return b"+OK\r\n".to_vec();
    }
    let slots = state
        .slots
        .lock()
        .expect("the slot map")
        .clone()
        .unwrap_or_default();
    encode_slots(&slots)
}

fn encode_slots(slots: &ClusterSlots) -> Vec<u8> {
    let mut out = format!("*{}\r\n", slots.len()).into_bytes();
    for (start, end, nodes) in slots {
        out.extend(
            format!(
                "*{}\r\n:{start}\r\n:{end}\r\n",
                nodes.len().saturating_add(2)
            )
            .into_bytes(),
        );
        for (host, port) in nodes {
            out.extend(bulk(host.as_bytes()));
            out.extend(format!(":{port}\r\n").into_bytes());
        }
    }
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

fn bulk(bytes: &[u8]) -> Vec<u8> {
    let mut out = format!("${}\r\n", bytes.len()).into_bytes();
    out.extend(bytes);
    out.extend(b"\r\n");
    out
}

struct Command {
    name: String,
    args: Vec<String>,
}

async fn read_command<R>(reader: &mut R) -> io::Result<Option<Command>>
where
    R: AsyncBufRead + Unpin,
{
    let mut line = String::new();
    if reader.read_line(&mut line).await? == 0 {
        return Ok(None);
    }
    let Some(count_text) = line.trim().strip_prefix('*') else {
        return Ok(None);
    };
    let count: usize = count_text
        .parse()
        .map_err(|_error| io::Error::other("the fake needs an array command"))?;
    let mut args = Vec::with_capacity(count);
    for _ in 0..count {
        let mut header = String::new();
        if reader.read_line(&mut header).await? == 0 {
            return Ok(None);
        }
        let Some(length_text) = header.trim().strip_prefix('$') else {
            return Ok(None);
        };
        let length: usize = length_text
            .parse()
            .map_err(|_error| io::Error::other("the fake needs a bulk string"))?;
        let mut buffer = vec![0_u8; length.saturating_add(2)];
        reader.read_exact(&mut buffer).await?;
        buffer.truncate(length);
        args.push(
            String::from_utf8(buffer).map_err(|_error| io::Error::other("a UTF-8 argument"))?,
        );
    }
    let name = args
        .first()
        .map(|arg| arg.to_ascii_uppercase())
        .unwrap_or_default();
    Ok(Some(Command { name, args }))
}

impl State {
    fn record(&self, command: &Command) {
        self.commands
            .lock()
            .expect("the command log")
            .push(Recorded {
                name: command.name.clone(),
                args: command.args.clone(),
            });
    }
}
