use std::io::{self, BufRead, BufReader, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

struct State {
    get: Option<Vec<u8>>,
    mget: Vec<Option<Vec<u8>>>,
    commands: Mutex<Vec<Vec<String>>>,
}

pub(crate) struct FakeServer {
    pub(crate) addr: SocketAddr,
    state: Arc<State>,
    stop: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
}

impl FakeServer {
    pub(crate) fn start(get: Option<Vec<u8>>, mget: Vec<Option<Vec<u8>>>) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let state = Arc::new(State {
            get,
            mget,
            commands: Mutex::new(Vec::new()),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::clone(&state);
        let stopping = Arc::clone(&stop);
        let accept =
            thread::Builder::new().spawn(move || accept_loop(&listener, &shared, &stopping))?;
        Ok(Self {
            addr,
            state,
            stop,
            accept: Some(accept),
        })
    }

    pub(crate) fn commands(&self) -> Vec<Vec<String>> {
        match self.state.commands.lock() {
            Ok(commands) => commands.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _wake = TcpStream::connect(self.addr);
        if let Some(accept) = self.accept.take() {
            let _joined = accept.join();
        }
    }
}

fn accept_loop(listener: &TcpListener, state: &Arc<State>, stop: &Arc<AtomicBool>) {
    while let Ok((stream, _peer)) = listener.accept() {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let shared = Arc::clone(state);
        let _spawned = thread::Builder::new().spawn(move || serve_connection(stream, &shared));
    }
}

fn serve_connection(stream: TcpStream, state: &State) {
    let Ok(cloned) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(cloned);
    let mut writer = stream;
    while let Ok(Some(command)) = read_command(&mut reader) {
        match state.commands.lock() {
            Ok(mut commands) => commands.push(command.args.clone()),
            Err(poisoned) => poisoned.into_inner().push(command.args.clone()),
        }
        if writer.write_all(&state.reply(&command)).is_err() {
            return;
        }
    }
}

struct Command {
    name: String,
    args: Vec<String>,
}

fn read_command<R>(reader: &mut R) -> io::Result<Option<Command>>
where
    R: BufRead,
{
    let Some(line) = read_line(reader)? else {
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
        let Some(header) = read_line(reader)? else {
            return Ok(None);
        };
        let Some(length_text) = header.trim().strip_prefix('$') else {
            return Ok(None);
        };
        let length: usize = length_text
            .parse()
            .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
        let mut buffer = vec![0_u8; length.saturating_add(2)];
        reader.read_exact(&mut buffer)?;
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

fn read_line<R>(reader: &mut R) -> io::Result<Option<String>>
where
    R: BufRead,
{
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    Ok(Some(line))
}

impl State {
    fn reply(&self, command: &Command) -> Vec<u8> {
        match command.name.as_str() {
            "SADD" => {
                if command.args.len() < 3 {
                    return b"-ERR wrong number of arguments for 'sadd' command\r\n".to_vec();
                }
                format!(":{}\r\n", command.args.len().saturating_sub(2)).into_bytes()
            }
            "EXPIRE" => b":1\r\n".to_vec(),
            "GET" => encode_bulk(self.get.as_deref()),
            "MGET" => encode_array(&self.mget),
            _ => b"+OK\r\n".to_vec(),
        }
    }
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
