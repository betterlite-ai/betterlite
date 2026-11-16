use std::io;
use std::time::Duration;

use http::Method;
use secrecy::SecretString;

use crate::commands::Client;
use crate::pi::{ProxyGet, ProxyResponse};

use super::{exposed, send};

impl ProxyGet for Client {
    fn get(
        &self,
        url: &str,
        headers: &[(String, SecretString)],
        _timeout: Duration,
    ) -> io::Result<ProxyResponse> {
        let response = send(self, url, Method::GET, exposed(headers))?;
        Ok(ProxyResponse {
            status: response.status,
            body: response.body,
        })
    }
}
