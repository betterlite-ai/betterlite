use std::io::{Error as IoError, ErrorKind, Result as IoResult};
use std::time::Duration;

use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio::time::sleep;

const AUTHENTICATION_OK: &[u8] = &[b'R', 0, 0, 0, 8, 0, 0, 0, 0];
const READY_FOR_QUERY: &[u8] = &[b'Z', 0, 0, 0, 5, b'I'];
const SET_COMPLETE: &[u8] = &[b'C', 0, 0, 0, 8, b'S', b'E', b'T', 0];
const STALL: Duration = Duration::from_secs(60);

// A startup handshake that finishes only after connect_delay, then never answers the DO
pub(super) async fn delayed_stalled_server(connect_delay: Duration) -> (SecretString, JoinSet<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut servers = JoinSet::new();
    servers.spawn(serve(listener, connect_delay));
    let url = SecretString::from(format!(
        "postgresql://stalled:stalled@{address}/betterlite?sslmode=disable"
    ));
    (url, servers)
}

async fn serve(listener: TcpListener, connect_delay: Duration) {
    let (mut socket, _peer) = listener.accept().await.unwrap();
    read_startup(&mut socket).await.unwrap();
    sleep(connect_delay).await;
    socket.write_all(AUTHENTICATION_OK).await.unwrap();
    socket.write_all(READY_FOR_QUERY).await.unwrap();
    while let Some(query) = read_query(&mut socket).await {
        if !query.starts_with("SET statement_timeout") {
            sleep(STALL).await;
            return;
        }
        socket.write_all(SET_COMPLETE).await.unwrap();
        socket.write_all(READY_FOR_QUERY).await.unwrap();
    }
}

async fn read_startup(socket: &mut TcpStream) -> IoResult<()> {
    let mut length = [0_u8; 4];
    socket.read_exact(&mut length).await?;
    let payload = message_body(i32::from_be_bytes(length))?;
    let mut startup = vec![0_u8; payload];
    socket.read_exact(&mut startup).await?;
    Ok(())
}

async fn read_query(socket: &mut TcpStream) -> Option<String> {
    let mut kind = [0_u8; 1];
    socket.read_exact(&mut kind).await.ok()?;
    if kind.first() != Some(&b'Q') {
        return None;
    }
    let mut length = [0_u8; 4];
    socket.read_exact(&mut length).await.ok()?;
    let payload = message_body(i32::from_be_bytes(length)).ok()?;
    let mut query = vec![0_u8; payload];
    socket.read_exact(&mut query).await.ok()?;
    let text = String::from_utf8(query).ok()?;
    Some(text.trim_end_matches('\0').to_owned())
}

fn message_body(size: i32) -> IoResult<usize> {
    size.checked_sub(4)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| IoError::new(ErrorKind::InvalidData, "message length below the header"))
}
