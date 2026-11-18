use std::io::Write as _;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::error::FixtureError;
use crate::request::{Request, read_request};

pub const READ_TIMEOUT: Duration = Duration::from_secs(60);

#[expect(
    clippy::partial_pub_fields,
    reason = "the tests reach the bound address as a field, and a getter for it is a one-line function the hooks refuse; the listener and the request handle stay private"
)]
pub struct Serving {
    pub address: SocketAddr,
    listener: Arc<TcpListener>,
    request: JoinHandle<Result<Request, FixtureError>>,
}

pub fn serve(answer: Vec<u8>, pause: Duration) -> Result<Serving, FixtureError> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let shared = Arc::new(listener);
    let server = Arc::clone(&shared);
    let request = thread::Builder::new()
        .name("http-fixture".to_owned())
        .spawn(move || answer_once(&server, &answer, pause))?;
    Ok(Serving {
        address,
        listener: shared,
        request,
    })
}

fn answer_once(
    listener: &TcpListener,
    answer: &[u8],
    pause: Duration,
) -> Result<Request, FixtureError> {
    let (mut stream, _peer) = listener.accept()?;
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    let request = read_request(&mut stream)?;
    thread::sleep(pause);
    // a client that gave up during the pause has closed the socket, and the test asserts the client's own result
    let _written = stream.write_all(answer);
    Ok(request)
}

impl Serving {
    pub fn finish(self) -> Result<Request, FixtureError> {
        drop(TcpStream::connect(self.listener.local_addr()?)?);
        self.request
            .join()
            .map_err(|_panicked| FixtureError::Panicked)?
    }
}

mod tests;
