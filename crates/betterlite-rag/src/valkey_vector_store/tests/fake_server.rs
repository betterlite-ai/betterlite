use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use tokio::io::{
    AsyncBufRead, AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

pub(crate) struct FakeValkey {
    pub(crate) addr: SocketAddr,
    commands: Arc<Mutex<Vec<Vec<Vec<u8>>>>>,
    tasks: JoinSet<()>,
}

impl Drop for FakeValkey {
    fn drop(&mut self) {
        self.tasks.abort_all();
    }
}

pub(crate) struct Handler {
    log: Arc<Mutex<Vec<Vec<Vec<u8>>>>>,
    reply: Vec<u8>,
}

impl FakeValkey {
    pub(crate) async fn start(reply: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("an ephemeral listener");
        let addr = listener.local_addr().expect("the listener address");
        let commands = Arc::new(Mutex::new(Vec::new()));
        let handler = Arc::new(Handler {
            log: Arc::clone(&commands),
            reply,
        });
        let mut tasks = JoinSet::new();
        tasks.spawn(accept_loop(listener, handler));
        Self {
            addr,
            commands,
            tasks,
        }
    }

    pub(crate) fn commands(&self) -> Vec<Vec<Vec<u8>>> {
        self.commands.lock().expect("the command log").clone()
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
    loop {
        match read_command(&mut reader).await {
            Ok(Some(command)) => {
                let answer = answer_for(&command, &handler.reply);
                handler.log.lock().expect("the command log").push(command);
                if write_half.write_all(&answer).await.is_err() {
                    return;
                }
            }
            Ok(None) | Err(_) => return,
        }
    }
}

fn answer_for(command: &[Vec<u8>], reply: &[u8]) -> Vec<u8> {
    let Some(name) = command.first() else {
        return b"+OK\r\n".to_vec();
    };
    if name.as_slice() == b"FT.SEARCH" {
        return reply.to_vec();
    }
    if name.as_slice() == b"PING" {
        return b"+PONG\r\n".to_vec();
    }
    b"+OK\r\n".to_vec()
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
