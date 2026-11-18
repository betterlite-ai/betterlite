use std::collections::BTreeMap;
use std::io::{Error as IoError, ErrorKind, Result as IoResult, Write as _};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_config::types::ProxyTrust;
use betterlite_http_fixture::request::read_request;
use betterlite_http_fixture::server::READ_TIMEOUT;
use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair, KeyUsagePurpose};
use rustls::crypto::aws_lc_rs;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::Acceptor;
use rustls::{ServerConfig, StreamOwned};

use crate::interactions::base::Params;

pub(crate) use betterlite_http_fixture::reply::reply;

pub(crate) fn params(text: &str) -> Params {
    serde_json::from_str(text).expect("the params are JSON")
}

pub(crate) fn policy(verification: Verification, ca_file: Option<&str>) -> TlsPolicy {
    TlsPolicy {
        verification,
        ca_file: ca_file.map(str::to_owned),
        certificate: None,
        http2: false,
        ecdh_curve: None,
        user_agent: "betterlite-test/0".to_owned(),
    }
}

// the policy the config crate resolves from an environment, built here instead of read from this process
pub(crate) fn proxy_policy(variables: &[(&str, &str)]) -> Arc<ProxyPolicy> {
    let environment: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &environment).unwrap())
}

pub(crate) fn tls_authority() -> Authority {
    let ca_key = KeyPair::generate().unwrap();
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    let issuer = Issuer::from_params(&ca_params, &ca_key);
    let leaf_key = KeyPair::generate().unwrap();
    let leaf_params = CertificateParams::new(vec!["localhost".to_owned()]).unwrap();
    let leaf = leaf_params.signed_by(&leaf_key, &issuer).unwrap();
    Authority {
        certificate: leaf.der().clone(),
        key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der())),
    }
}

pub(crate) fn spawn_tls_server(
    authority: Authority,
    listener: TcpListener,
) -> thread::JoinHandle<IoResult<String>> {
    thread::Builder::new()
        .spawn(move || authority.answer(&listener))
        .unwrap()
}

pub(crate) struct Authority {
    certificate: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
}

impl Authority {
    fn answer(&self, listener: &TcpListener) -> IoResult<String> {
        let (mut stream, _peer) = listener.accept()?;
        stream.set_read_timeout(Some(READ_TIMEOUT))?;
        stream.set_write_timeout(Some(READ_TIMEOUT))?;
        let mut acceptor = Acceptor::default();
        let accepted = loop {
            if acceptor.read_tls(&mut stream)? == 0 {
                return Err(IoError::from(ErrorKind::UnexpectedEof));
            }
            if let Some(accepted) = acceptor
                .accept()
                .map_err(|(error, _alert)| IoError::other(error))?
            {
                break accepted;
            }
        };
        let config = ServerConfig::builder_with_provider(Arc::new(aws_lc_rs::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(IoError::other)?
            .with_no_client_auth()
            .with_single_cert(vec![self.certificate.clone()], self.key.clone_key())
            .map_err(IoError::other)?;
        let connection = accepted
            .into_connection(Arc::new(config))
            .map_err(|(error, _alert)| IoError::other(error))?;
        let mut secured = StreamOwned::new(connection, stream);
        let request = read_request(&mut secured).map_err(IoError::other)?;
        secured
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}")?;
        secured.flush()?;
        Ok(request.head)
    }
}
