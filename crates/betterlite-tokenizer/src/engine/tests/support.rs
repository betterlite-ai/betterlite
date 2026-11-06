use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::future::pending;
use std::path::Path;
use std::str::from_utf8;
use std::sync::{Arc, Mutex};

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::huggingface_hub::{
    OfflineSetting, Settings as HubSettings, TransferSetting,
};
use betterlite_config::sections::huggingface_oidc::Settings as OidcSettings;
use betterlite_config::types::ProxyTrust;
use betterlite_http::hub_oauth::clock::{HubClock, SystemHubClock};
use betterlite_http::hub_token::{HubTokenFileReader, HubTokenResolver, TokioHubTokenFileReader};
use betterlite_http::proxy::ProxiedClient;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

pub(crate) struct Request {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) headers: BTreeMap<String, String>,
}

impl Request {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}

pub(crate) enum Reply {
    Response {
        status: u16,
        headers: Vec<(&'static str, String)>,
        body: Vec<u8>,
    },
    HeadersThenStall {
        headers: Vec<(&'static str, String)>,
    },
    Stall,
}

pub(crate) struct Server {
    pub(crate) endpoint: String,
    pub(crate) requests: Arc<Mutex<Vec<Request>>>,
    pub(crate) tasks: Arc<Mutex<JoinSet<()>>>,
}

impl Server {
    pub(crate) async fn start<H>(handler: H) -> Self
    where
        H: Fn(&Request) -> Reply + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let requests: Arc<Mutex<Vec<Request>>> = Arc::default();
        let tasks: Arc<Mutex<JoinSet<()>>> = Arc::default();
        let shared_handler = Arc::new(handler);
        tasks.lock().unwrap().spawn(accept_loop(
            listener,
            shared_handler,
            Arc::clone(&requests),
            Arc::clone(&tasks),
        ));
        Self {
            endpoint,
            requests,
            tasks,
        }
    }

    pub(crate) fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    pub(crate) fn last_path(&self) -> Option<String> {
        self.requests
            .lock()
            .unwrap()
            .last()
            .map(|request| request.path.clone())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.tasks.lock().unwrap().abort_all();
    }
}

async fn accept_loop(
    listener: TcpListener,
    handler: Arc<dyn Fn(&Request) -> Reply + Send + Sync>,
    requests: Arc<Mutex<Vec<Request>>>,
    tasks: Arc<Mutex<JoinSet<()>>>,
) {
    loop {
        let Ok((socket, _)) = listener.accept().await else {
            return;
        };
        let connection_handler = Arc::clone(&handler);
        let connection_requests = Arc::clone(&requests);
        tasks.lock().unwrap().spawn(async move {
            serve(socket, connection_handler, connection_requests).await;
        });
    }
}

async fn serve(
    mut socket: TcpStream,
    handler: Arc<dyn Fn(&Request) -> Reply + Send + Sync>,
    requests: Arc<Mutex<Vec<Request>>>,
) {
    let Some(request) = read_request(&mut socket).await else {
        return;
    };
    let reply = handler(&request);
    let is_head = request.method == "HEAD";
    requests.lock().unwrap().push(request);
    match reply {
        Reply::Response {
            status,
            headers,
            body,
        } => write_response(&mut socket, is_head, status, headers, &body).await,
        Reply::HeadersThenStall { headers } => {
            let mut head = "HTTP/1.1 200 OK\r\n".to_owned();
            for (name, value) in headers {
                write!(head, "{name}: {value}\r\n").unwrap();
            }
            head.push_str("Content-Length: 1024\r\n\r\n");
            let _head_written = socket.write_all(head.as_bytes()).await;
            pending::<()>().await;
        }
        Reply::Stall => pending::<()>().await,
    }
}

async fn write_response(
    socket: &mut TcpStream,
    is_head: bool,
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: &[u8],
) {
    let mut head = format!("HTTP/1.1 {status} {}\r\n", reason(status));
    for (name, value) in headers {
        write!(head, "{name}: {value}\r\n").unwrap();
    }
    write!(
        head,
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    let _head_written = socket.write_all(head.as_bytes()).await;
    if !is_head {
        let _body_written = socket.write_all(body).await;
    }
    let _shutdown = socket.shutdown().await;
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        304 => "Not Modified",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

async fn read_request(socket: &mut TcpStream) -> Option<Request> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let read = socket.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(chunk.get(..read)?);
        if buffer.len() > 16 * 1024 || buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let text = from_utf8(&buffer).ok()?;
    let mut lines = text.split("\r\n");
    let mut parts = lines.next()?.split_whitespace();
    let method = parts.next()?.to_owned();
    let path = parts.next()?.to_owned();
    let mut headers = BTreeMap::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':')?;
        headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
    }
    Some(Request {
        method,
        path,
        headers,
    })
}

pub(crate) fn client() -> Arc<ProxiedClient> {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    Arc::new(ProxiedClient::outbound(&policy).unwrap())
}

pub(crate) fn hub_settings(endpoint: &str, directory: &Path) -> HubSettings {
    HubSettings {
        endpoint: endpoint.to_owned(),
        offline: OfflineSetting::Online,
        etag_timeout: 10,
        download_timeout: 10,
        disable_implicit_token: false,
        env_token: None,
        hf_home: directory.to_path_buf(),
        hub_cache: directory.to_path_buf(),
        token_path: directory.join("token"),
        stored_tokens_path: directory.join("stored_tokens"),
        transfer: TransferSetting::Standard,
    }
}

pub(crate) fn resolver(settings: &HubSettings) -> Arc<HubTokenResolver> {
    let oidc = OidcSettings {
        resource: None,
        id_token: None,
        github_actions: false,
        request_url: None,
        request_token: None,
        device_code_client_id: String::new(),
    };
    let clock: Arc<dyn HubClock> = Arc::new(SystemHubClock::default());
    let reader: Arc<dyn HubTokenFileReader> = Arc::new(TokioHubTokenFileReader);
    Arc::new(HubTokenResolver::new(
        settings.clone(),
        oidc,
        client(),
        reader,
        clock,
    ))
}
