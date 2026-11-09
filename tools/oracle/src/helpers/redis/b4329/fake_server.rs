mod apply;
mod command;
mod pop;
mod push;
mod replies;
mod trim;

use std::collections::BTreeMap;
use std::io;
use std::mem;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::io::AsyncWriteExt as _;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

use self::apply::apply;
use self::command::{Command, read_command};

type Lists = BTreeMap<String, Vec<Vec<u8>>>;

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
    pub(crate) async fn start(initial: Lists) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let lists = Arc::new(Mutex::new(initial));
        let shared = Arc::clone(&lists);
        let mut tasks = JoinSet::new();
        tasks.spawn(async move { accept_loop(listener, shared).await });
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

fn locked(lists: &Mutex<Lists>) -> MutexGuard<'_, Lists> {
    match lists.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[derive(Default)]
struct Session {
    in_multi: bool,
    queued: Vec<Command>,
}

async fn accept_loop(listener: TcpListener, lists: Arc<Mutex<Lists>>) {
    let mut connections = JoinSet::new();
    while let Ok((stream, _peer)) = listener.accept().await {
        connections.spawn(serve(stream, Arc::clone(&lists)));
        while connections.try_join_next().is_some() {}
    }
}

async fn serve(stream: TcpStream, lists: Arc<Mutex<Lists>>) {
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut reader = tokio::io::BufReader::new(read_half);
    let mut session = Session::default();
    while let Ok(Some(command)) = read_command(&mut reader).await {
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
