use std::collections::BTreeMap;
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use tokio::io::{
    AsyncBufRead, AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

type CommandLog = Arc<Mutex<Vec<Vec<Vec<u8>>>>>;

pub(crate) struct FakeValkey {
    pub(crate) addr: SocketAddr,
    commands: CommandLog,
    tasks: JoinSet<()>,
}

impl Drop for FakeValkey {
    fn drop(&mut self) {
        self.tasks.abort_all();
    }
}

struct Handler {
    log: CommandLog,
    replies: BTreeMap<String, Vec<u8>>,
}

impl FakeValkey {
    pub(crate) async fn start(replies: &[(&str, &str)]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("an ephemeral listener");
        let addr = listener.local_addr().expect("the listener address");
        let commands = CommandLog::default();
        let handler = Arc::new(Handler {
            log: Arc::clone(&commands),
            replies: replies
                .iter()
                .map(|(name, reply)| ((*name).to_owned(), reply.as_bytes().to_vec()))
                .collect(),
        });
        let mut tasks = JoinSet::new();
        tasks.spawn(accept_loop(listener, handler));
        Self {
            addr,
            commands,
            tasks,
        }
    }

    pub(crate) fn url(&self) -> String {
        format!("redis://{}:{}", self.addr.ip(), self.addr.port())
    }

    pub(crate) fn received(&self, names: &[&[u8]]) -> Vec<Vec<Vec<u8>>> {
        self.commands
            .lock()
            .expect("the command log")
            .iter()
            .filter(|command| {
                command
                    .first()
                    .is_some_and(|first| names.contains(&first.as_slice()))
            })
            .cloned()
            .collect()
    }
}

async fn accept_loop(listener: TcpListener, handler: Arc<Handler>) {
    let mut connections = JoinSet::new();
    while let Ok((socket, _peer)) = listener.accept().await {
        let connection = Arc::clone(&handler);
        connections.spawn(async move { serve(socket, connection).await });
        while connections.try_join_next().is_some() {}
    }
}

async fn serve(socket: TcpStream, handler: Arc<Handler>) {
    let (read_half, mut write_half) = socket.into_split();
    let mut reader = BufReader::new(read_half);
    while let Ok(Some(command)) = read_command(&mut reader).await {
        let name = command
            .first()
            .map(|first| String::from_utf8(first.clone()).expect("a text command name"))
            .unwrap_or_default();
        let answer = handler
            .replies
            .get(&name)
            .cloned()
            .unwrap_or_else(|| b"+OK\r\n".to_vec());
        handler.log.lock().expect("the command log").push(command);
        if write_half.write_all(&answer).await.is_err() {
            return;
        }
    }
}

async fn read_command<R>(reader: &mut R) -> io::Result<Option<Vec<Vec<u8>>>>
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
        args.push(buffer);
    }
    Ok(Some(args))
}
