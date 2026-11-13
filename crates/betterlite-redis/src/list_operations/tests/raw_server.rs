use std::collections::VecDeque;
use std::io::{self, BufRead as _, BufReader, Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::str;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

pub(crate) struct RawServer {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    replies: Arc<Mutex<VecDeque<Vec<u8>>>>,
    accept: Option<JoinHandle<()>>,
}

impl RawServer {
    pub(crate) fn start(scripted: Vec<Vec<u8>>) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let replies = Arc::new(Mutex::new(VecDeque::from(scripted)));
        let shared = Arc::clone(&replies);
        let stopping = Arc::clone(&stop);
        let accept =
            thread::Builder::new().spawn(move || accept_loop(&listener, &shared, &stopping))?;
        Ok(Self {
            addr,
            stop,
            replies,
            accept: Some(accept),
        })
    }

    pub(crate) fn host(&self) -> String {
        self.addr.ip().to_string()
    }

    pub(crate) const fn port(&self) -> u16 {
        self.addr.port()
    }

    pub(crate) fn pending(&self) -> usize {
        match self.replies.lock() {
            Ok(queued) => queued.len(),
            Err(poisoned) => poisoned.into_inner().len(),
        }
    }
}

impl Drop for RawServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _wake = TcpStream::connect(self.addr);
        if let Some(accept) = self.accept.take() {
            let _joined = accept.join();
        }
    }
}

pub(crate) fn bulk(bytes: &[u8]) -> Vec<u8> {
    let mut out = format!("${}\r\n", bytes.len()).into_bytes();
    out.extend_from_slice(bytes);
    out.extend_from_slice(b"\r\n");
    out
}

pub(crate) fn array(values: &[Vec<u8>]) -> Vec<u8> {
    let mut out = format!("*{}\r\n", values.len()).into_bytes();
    for value in values {
        out.extend(bulk(value));
    }
    out
}

fn accept_loop(
    listener: &TcpListener,
    replies: &Arc<Mutex<VecDeque<Vec<u8>>>>,
    stop: &Arc<AtomicBool>,
) {
    while let Ok((stream, _peer)) = listener.accept() {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let shared = Arc::clone(replies);
        let _spawned = thread::Builder::new().spawn(move || serve(stream, &shared));
    }
}

fn serve(stream: TcpStream, replies: &Mutex<VecDeque<Vec<u8>>>) {
    let Ok(cloned) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(cloned);
    let mut writer = stream;
    loop {
        let Ok(Some(name)) = read_command(&mut reader) else {
            return;
        };
        if name != "LPOP" {
            if writer.write_all(b"+OK\r\n").is_err() {
                return;
            }
            continue;
        }
        let next = match replies.lock() {
            Ok(mut queued) => queued.pop_front(),
            Err(poisoned) => poisoned.into_inner().pop_front(),
        };
        let Some(reply) = next else {
            return;
        };
        if writer.write_all(&reply).is_err() {
            return;
        }
    }
}

fn read_command(reader: &mut BufReader<TcpStream>) -> io::Result<Option<String>> {
    let mut header = String::new();
    if reader.read_line(&mut header)? == 0 {
        return Ok(None);
    }
    let Some(count_text) = header.trim().strip_prefix('*') else {
        return Ok(None);
    };
    let count: usize = count_text
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut name = String::new();
    for index in 0..count {
        let mut length_line = String::new();
        if reader.read_line(&mut length_line)? == 0 {
            return Ok(None);
        }
        let Some(length_text) = length_line.trim().strip_prefix('$') else {
            return Ok(None);
        };
        let length: usize = length_text
            .parse()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let mut buffer = vec![0_u8; length.saturating_add(2)];
        reader.read_exact(&mut buffer)?;
        if index == 0 {
            buffer.truncate(length);
            let Ok(text) = str::from_utf8(&buffer) else {
                return Ok(None);
            };
            name = text.to_ascii_uppercase();
        }
    }
    Ok(Some(name))
}
