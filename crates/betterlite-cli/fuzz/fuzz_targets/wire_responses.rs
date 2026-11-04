#![no_main]

use betterlite_cli::pkce_login::discovery::discover_cli_auth;
use betterlite_cli::pkce_login::models::{CliAuthContract, PkceFailure};
use betterlite_cli::pkce_login::oauth::register_client;
use betterlite_cli::pkce_login::wire::{error_detail, parse_token};
use betterlite_cli::pkce_login::{Http, HttpBody, HttpResponse};
use libfuzzer_sys::fuzz_target;

const BASE_URL: &str = "https://example.test";

struct FuzzHttp<'bytes> {
    body: &'bytes [u8],
}

impl Http for FuzzHttp<'_> {
    fn get(&self, _url: &str, _timeout_seconds: u64) -> Result<HttpResponse, PkceFailure> {
        Ok(HttpResponse {
            status: 200,
            headers: Vec::new(),
            body: self.body.to_vec(),
        })
    }

    fn post(
        &self,
        _url: &str,
        _body: HttpBody,
        _timeout_seconds: u64,
        _follow_redirects: bool,
    ) -> Result<HttpResponse, PkceFailure> {
        Ok(HttpResponse {
            status: 200,
            headers: Vec::new(),
            body: self.body.to_vec(),
        })
    }
}

fn fixed_contract() -> CliAuthContract {
    CliAuthContract {
        contract_version: 1,
        issuer: BASE_URL.to_owned(),
        authorization_endpoint: format!("{BASE_URL}/oauth/authorize"),
        token_endpoint: format!("{BASE_URL}/oauth/token"),
        registration_endpoint: format!("{BASE_URL}/oauth/register"),
        revocation_endpoint: format!("{BASE_URL}/oauth/revoke"),
        resource: BASE_URL.to_owned(),
        supported_methods: vec!["S256".to_owned()],
    }
}

fuzz_target!(|bytes: &[u8]| {
    let http = FuzzHttp { body: bytes };
    let _token = parse_token(bytes);
    let _detail = error_detail(bytes);
    let _discovered = discover_cli_auth(BASE_URL, &http);
    let contract = fixed_contract();
    let _registered = register_client(&contract, "http://127.0.0.1:0/callback", &http);
});
