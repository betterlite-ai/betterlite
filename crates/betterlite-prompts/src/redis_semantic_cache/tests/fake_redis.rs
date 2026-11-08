use std::collections::{BTreeMap, VecDeque};
use std::net::SocketAddr;
use std::str;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

use crate::redis_semantic_cache::live::LiveSemanticIndexConnector;
use crate::redis_semantic_cache::ports::{SemanticIndexConnector as _, SemanticRedisIndex};

type Script = Arc<Mutex<BTreeMap<String, VecDeque<String>>>>;
type CommandLog = Arc<Mutex<Vec<Vec<Vec<u8>>>>>;

pub(crate) struct FakeRedis {
    addr: SocketAddr,
    script: Script,
    log: CommandLog,
    tasks: JoinSet<()>,
}

impl Drop for FakeRedis {
    fn drop(&mut self) {
        self.tasks.abort_all();
    }
}

impl FakeRedis {
    pub(crate) async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a free local port");
        let addr = listener.local_addr().expect("a bound address");
        let script = Script::default();
        let log = CommandLog::default();
        let mut tasks = JoinSet::new();
        tasks.spawn(accept_loop(listener, Arc::clone(&script), Arc::clone(&log)));
        Self {
            addr,
            script,
            log,
            tasks,
        }
    }

    pub(crate) fn reply(&self, command: &str, wire: &str) {
        self.script
            .lock()
            .expect("the reply script")
            .entry(command.to_owned())
            .or_default()
            .push_back(wire.to_owned());
    }

    pub(crate) fn commands(&self, name: &str) -> Vec<Vec<Vec<u8>>> {
        self.log
            .lock()
            .expect("the command log")
            .iter()
            .filter(|command| {
                command
                    .first()
                    .is_some_and(|first| first == name.as_bytes())
            })
            .cloned()
            .collect()
    }

    pub(crate) fn arrival_order(&self, names: &[&str]) -> Vec<String> {
        self.log
            .lock()
            .expect("the command log")
            .iter()
            .filter_map(|command| str::from_utf8(command.first()?).ok())
            .filter(|name| names.contains(name))
            .map(str::to_owned)
            .collect()
    }

    pub(crate) async fn connect(&self) -> Arc<dyn SemanticRedisIndex> {
        LiveSemanticIndexConnector
            .connect(&format!("redis://:pw@{}", self.addr))
            .await
            .expect("a connection to the fake server")
    }
}

async fn accept_loop(listener: TcpListener, script: Script, log: CommandLog) {
    let mut connections = JoinSet::new();
    while let Ok((stream, _)) = listener.accept().await {
        connections.spawn(serve(stream, Arc::clone(&script), Arc::clone(&log)));
    }
}

async fn serve(stream: TcpStream, script: Script, log: CommandLog) {
    let mut reader = BufReader::new(stream);
    while let Some(command) = read_command(&mut reader).await {
        let Some(first) = command.first() else {
            continue;
        };
        let Ok(raw_name) = str::from_utf8(first) else {
            return;
        };
        let name = raw_name.to_uppercase();
        let reply = script
            .lock()
            .expect("the reply script")
            .get_mut(&name)
            .and_then(VecDeque::pop_front)
            .unwrap_or_else(|| "+OK\r\n".to_owned());
        log.lock().expect("the command log").push(command);
        if reader.get_mut().write_all(reply.as_bytes()).await.is_err() {
            return;
        }
    }
}

async fn read_command(reader: &mut BufReader<TcpStream>) -> Option<Vec<Vec<u8>>> {
    let count: usize = read_line(reader).await?.strip_prefix('*')?.parse().ok()?;
    let mut arguments = Vec::new();
    for _ in 0..count {
        let size: usize = read_line(reader).await?.strip_prefix('$')?.parse().ok()?;
        let mut bytes = vec![0_u8; size.saturating_add(2)];
        reader.read_exact(&mut bytes).await.ok()?;
        bytes.truncate(size);
        arguments.push(bytes);
    }
    Some(arguments)
}

async fn read_line(reader: &mut BufReader<TcpStream>) -> Option<String> {
    let mut line = String::new();
    if reader.read_line(&mut line).await.ok()? == 0 {
        return None;
    }
    Some(line.trim_end().to_owned())
}
