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

impl FakeValkey {
    pub(crate) async fn start(reply: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("an ephemeral listener");
        let addr = listener.local_addr().expect("the listener address");
        let commands = Arc::new(Mutex::new(Vec::new()));
        let mut tasks = JoinSet::new();
        let log = Arc::clone(&commands);
        tasks.spawn(accept_loop(listener, log, reply));
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

async fn accept_loop(listener: TcpListener, log: Arc<Mutex<Vec<Vec<Vec<u8>>>>>, reply: Vec<u8>) {
    let mut connections = JoinSet::new();
    loop {
        let Ok((socket, _peer)) = listener.accept().await else {
            return;
        };
        let connection_log = Arc::clone(&log);
        let connection_reply = reply.clone();
        connections.spawn(async move { serve(socket, connection_log, connection_reply).await });
        while connections.try_join_next().is_some() {}
    }
}

async fn serve(socket: TcpStream, log: Arc<Mutex<Vec<Vec<Vec<u8>>>>>, reply: Vec<u8>) {
    let (read_half, mut write_half) = socket.into_split();
    let mut reader = BufReader::new(read_half);
    while let Ok(Some(command)) = read_command(&mut reader).await {
        let searched = command
            .first()
            .is_some_and(|name| name.as_slice() == b"FT.SEARCH");
        let pinged = command
            .first()
            .is_some_and(|name| name.as_slice() == b"PING");
        log.lock().expect("the command log").push(command);
        let answer = if searched {
            reply.clone()
        } else if pinged {
            b"+PONG\r\n".to_vec()
        } else {
            b"+OK\r\n".to_vec()
        };
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
