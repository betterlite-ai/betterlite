use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};

use serde::Deserialize;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

pub(crate) const CLOUD_PLATFORM: &str = "https://www.googleapis.com/auth/cloud-platform";

// a throwaway test key, never a real service account
const SERVICE_ACCOUNT_KEY: &str = "-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDhMvkxt/lVX1L0
mbP2VBxO8tLN/xjlzM7h18lvswMDNMDqqI6wlsA5m1o6RH5Hp67di20fws2l+Epj
KtF4TyL5kXcD/SdNOcDg8QMbSv9vP2JeQQwNkIUIc3tq8kZspSgBA0YhMXMnp5tj
xbHWKQYyepR5tJARyxHjQRxr+UQk+2IstIHdBdUXdUsyB5onY3ooZ5oHX2aP57Kc
6gqnZBE1KfGv7XGOQ0UG6Yr1aVVszc7U5I7h2uBDtMXMLqoXLMm+0Hss8wnof1vk
IRylPOTFlVnGIEOUPKT2zFqI24AT2gi1+/KNjS0lzEdkYkr3Eog3ZtCsKyFwHBE9
FvxuRi7ZAgMBAAECggEAF67o6pvmvp35ByzP8oirdnkOhuf8ShSVMEalklfw6eon
HLaP1Q3xls1Z+pU/85C0sOtsFUJDJuB7ED1NTLoD21u7OMHrm3FGsdFe6oSjYuN3
WKK+HgIUH4ynjtp+Yggnch3SRwTgj6vOiXpdxsBHPNFL4pXE7f4dejkbgFZ1Xm8Y
Bhnpbz5+KFJc+0xYHlXNN36yKihb64fu8E0sR3T1+ShWQtK6xOSX2heKWaAKulh/
/b1pGM9dDIwEk3P2f/03nGMI7wJz7R/6m2nKPsVUZRhgutMeMRQLu90r1e5RDqm1
CBDJ5OkXOpyEntT+C8wYEqzIfsjleCjLyaEOtZ1/tQKBgQDwaNNGc61HyR57SKW5
iA0vhnWr2R/8HyHleUfk2oka0CVzXRSe4xT/VD/q9KQqns4hXl/oY6zksR16jQRZ
KXKpkY5DaqqjUFLmeiU3OaN1LsDFFuBF83SNYc2Y44AQJ4xDvY0xOSm5d8DXUfQN
WSmPVgLJVZyoIuafZBiE7K6PvQKBgQDvzaHdUBKbEw7O4lwFbLRh7c3NgAbcx9K4
D55M43nTjyd8PzeXHUVnQlh85+XGa4+V58C1J//pbnd+rCt1xGNPIZS87yDMmvHB
KYacWSCMG8+2VooGl0E91WLn00dvoG3V14VsXdUrd04XhNrx1+Kk48XzRz0vlGJA
KYHw6VpvTQKBgQCY8Uc/scXSPmuq6LutvZUN4A+qRRZpEfFBiJepSzIgIb1BmDzP
HA42pYWIVxy9lIv0hJKlSAws7VSgq2fRxWGR6OzFjO2Duk0THS/h40RO/3BZskT5
m3lRnJ/snQbSwCYmzkctwnz95eaD1Tpa55dd/c0GAvYHrTRkwIwsvFoNTQKBgQCR
TfmZn65WdvrPE0wNIWw/tpewwLXny9D8JMKX9nJYZfLYD3pAv8TRJ9t0fBO34m8T
t2UGoCEqMfyILgz7hC9kwIVe4kruj6iTkfT2Ae991lBLPS7PhZFLxWPz8wrjMcO8
AqbfHhBQES+pgqAkUNsNbNMCcZU6ol/SXxRUhSF+NQKBgCGd9YNWV3x+he8W3YeF
MRO8FmxHakwQpTZBM8JKoXjusL6Ds1lJ33Rn7Y4qe1oKkQIjDgD6PfMrBgwRym7d
D+AQuMla/k4dewE/xyJ0srlTXkHMJycAzBBILkHLDNERzuBykAOYXaRdYMwzQGMh
mk8GsVreg0+vfw3SGmmxjmVv
-----END PRIVATE KEY-----
";

#[derive(Deserialize)]
pub(crate) struct JwtClaims {
    pub(crate) iss: String,
    pub(crate) scope: String,
}

#[derive(Deserialize)]
pub(crate) struct RefreshRequest {
    pub(crate) grant_type: String,
    pub(crate) refresh_token: String,
    pub(crate) scope: String,
}

#[derive(Clone)]
pub(crate) struct CapturedRequest {
    pub(crate) head: String,
    pub(crate) body: String,
}

pub(crate) struct Upstream {
    pub(crate) address: SocketAddr,
    captured: Arc<Mutex<Vec<CapturedRequest>>>,
}

impl Upstream {
    pub(crate) fn captured(&self) -> Vec<CapturedRequest> {
        self.captured
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(crate) fn service_account_document(client_email: &str) -> String {
    let key = SERVICE_ACCOUNT_KEY.replace('\n', "\\n");
    format!(
        r#"{{"type":"service_account","project_id":"project","private_key_id":"key-id","private_key":"{key}","client_email":"{client_email}"}}"#
    )
}

pub(crate) fn authorized_user_document(refresh_token: &str, server: &Upstream) -> String {
    let address = server.address;
    format!(
        r#"{{"type":"authorized_user","client_id":"client","client_secret":"client-secret","refresh_token":"{refresh_token}","token_uri":"http://{address}/token"}}"#
    )
}

pub(crate) fn refresh_requests(server: &Upstream) -> Vec<RefreshRequest> {
    server
        .captured()
        .iter()
        .map(|request| serde_json::from_str(&request.body).expect("the refresh request is JSON"))
        .collect()
}

pub(crate) async fn token_server(access_token: &str) -> Upstream {
    upstream(
        200,
        format!(r#"{{"access_token":"{access_token}","expires_in":3599,"token_type":"Bearer"}}"#),
    )
    .await
}

// answers every request with the given status and body, and keeps what each request sent
pub(crate) async fn upstream(status: u16, reply: String) -> Upstream {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the listener binds");
    let address = listener.local_addr().expect("the listener has an address");
    let captured = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&captured);
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.expect("a client connects");
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                head.push(socket.read_u8().await.expect("the request has a head"));
            }
            let head = String::from_utf8(head).expect("the head is text");
            let length: usize = head
                .lines()
                .filter_map(|line| line.split_once(": "))
                .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .map_or(0, |(_, value)| value.trim().parse().expect("a length"));
            let mut body = vec![0; length];
            socket
                .read_exact(&mut body)
                .await
                .expect("the request has its body");
            seen.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(CapturedRequest {
                    head,
                    body: String::from_utf8(body).expect("the body is text"),
                });
            let response = format!(
                "HTTP/1.1 {status} Reply\r\ncontent-type: application/json\r\nx-reply: marker\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
                reply.len()
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("the reply is written");
        }
    });
    Upstream { address, captured }
}
