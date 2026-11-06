use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use tokio::time::sleep;

use crate::client::sessions::stdio::StdioTransport;
use crate::client::sessions::transport::{StdioServerParameters, TransportContext as _};

fn parameters() -> StdioServerParameters {
    StdioServerParameters {
        command: "/bin/sleep".to_owned(),
        args: vec!["60".to_owned()],
        env: BTreeMap::from([("PATH".to_owned(), "/usr/bin:/bin".to_owned())]),
    }
}

fn process_exists(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

#[tokio::test]
async fn the_stdio_server_dies_when_its_owner_exits() {
    let (mut transport, pid) = StdioTransport::spawn(parameters()).unwrap();
    let streams = transport.enter().await.unwrap();
    drop(streams);
    transport.exit().await.unwrap();
    assert!(!process_exists(pid));
}

#[tokio::test]
async fn the_stdio_server_dies_when_its_owner_is_dropped() {
    let (mut transport, pid) = StdioTransport::spawn(parameters()).unwrap();
    let streams = transport.enter().await.unwrap();
    drop(transport);
    drop(streams);
    sleep(Duration::from_millis(200)).await;
    assert!(!process_exists(pid));
}
