use secrecy::{ExposeSecret as _, SecretString};

#[derive(Clone, Debug)]
pub struct RequestHeader {
    pub name: &'static str,
    pub value: SecretString,
}

pub(super) fn request_headers(
    auth_method: &str,
    access_token: &SecretString,
) -> Vec<RequestHeader> {
    let mut headers = vec![
        RequestHeader {
            name: "Accept",
            value: SecretString::from("application/json"),
        },
        RequestHeader {
            name: "Content-Type",
            value: SecretString::from("application/json"),
        },
    ];
    if auth_method == "oauth" {
        headers.push(RequestHeader {
            name: "Authorization",
            value: SecretString::from(format!("Bearer {}", access_token.expose_secret())),
        });
    } else {
        headers.push(RequestHeader {
            name: "Private-Token",
            value: access_token.clone(),
        });
    }
    headers
}

mod tests;
