use std::collections::BTreeMap;
use std::fs;

use redis::cluster::ClusterClientBuilder;
use redis::{Client, ClientTlsConfig, ConnectionAddr, ConnectionInfo, TlsCertificates, TlsMode};
use secrecy::{ExposeSecret as _, SecretString};

use super::LiveClientError;
use crate::redis_connection_settings::model::scalars::PassthroughValue;

const TLS_KEYS: [&str; 5] = [
    "ssl_ca_certs",
    "ssl_certfile",
    "ssl_keyfile",
    "ssl_cert_reqs",
    "ssl_check_hostname",
];

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum TlsState {
    #[default]
    Disabled,
    Enabled,
}

#[derive(Default)]
pub(crate) struct TlsSettings {
    pub(crate) state: TlsState,
    pub(crate) insecure: bool,
    pub(crate) accept_invalid_hostnames: bool,
    pub(crate) certificates: Option<TlsCertificates>,
}

pub(crate) fn from_passthrough(
    map: &BTreeMap<String, PassthroughValue>,
) -> Result<TlsSettings, LiveClientError> {
    values(&Values {
        passthrough: map,
        query: None,
    })
}

pub(crate) fn from_url(
    url: &SecretString,
    passthrough: &BTreeMap<String, PassthroughValue>,
) -> Result<TlsSettings, LiveClientError> {
    let Ok(parsed) = url::Url::parse(url.expose_secret()) else {
        return Err(LiveClientError::InvalidSetting { field: "url" });
    };
    let query: BTreeMap<String, String> = parsed.query_pairs().into_owned().collect();
    let mut settings = values(&Values {
        passthrough,
        query: Some(&query),
    })?;
    if matches!(parsed.scheme(), "rediss" | "valkeys") {
        settings.state = TlsState::Enabled;
    }
    Ok(settings)
}

fn values(source: &Values<'_>) -> Result<TlsSettings, LiveClientError> {
    let certificates = read_certificates(
        source.text("ssl_ca_certs")?,
        source.text("ssl_certfile")?,
        source.text("ssl_keyfile")?,
    )?;
    Ok(TlsSettings {
        state: TlsState::Disabled,
        insecure: source.cert_reqs_none()?,
        accept_invalid_hostnames: source
            .flag("ssl_check_hostname")?
            .is_some_and(|checked| !checked),
        certificates,
    })
}

struct Values<'data> {
    passthrough: &'data BTreeMap<String, PassthroughValue>,
    query: Option<&'data BTreeMap<String, String>>,
}

impl Values<'_> {
    fn text(&self, key: &'static str) -> Result<Option<String>, LiveClientError> {
        match self.passthrough.get(key) {
            Some(PassthroughValue::Text(text)) => Ok(Some(text.clone())),
            Some(_) => Err(LiveClientError::InvalidSetting { field: key }),
            None => Ok(self.query.and_then(|query| query.get(key).cloned())),
        }
    }

    fn flag(&self, key: &'static str) -> Result<Option<bool>, LiveClientError> {
        let text = match self.passthrough.get(key) {
            Some(PassthroughValue::Bool(flag)) => return Ok(Some(*flag)),
            Some(PassthroughValue::Text(value)) => Some(value),
            Some(_) => return Err(LiveClientError::InvalidSetting { field: key }),
            None => self.query.and_then(|query| query.get(key)),
        };
        text.map_or(Ok(None), |value| {
            betterlite_env::flag(key, value)
                .map_err(|_source| LiveClientError::InvalidSetting { field: key })
        })
    }

    fn cert_reqs_none(&self) -> Result<bool, LiveClientError> {
        match self.passthrough.get("ssl_cert_reqs") {
            Some(PassthroughValue::Text(text)) => Ok(text.eq_ignore_ascii_case("none")),
            Some(PassthroughValue::Number(number)) => Ok(number.as_u64() == Some(0)),
            Some(_) => Err(LiveClientError::InvalidSetting {
                field: "ssl_cert_reqs",
            }),
            None => Ok(self
                .query
                .and_then(|query| query.get("ssl_cert_reqs"))
                .is_some_and(|text| text.eq_ignore_ascii_case("none"))),
        }
    }
}

fn read_certificates(
    ca: Option<String>,
    cert: Option<String>,
    key: Option<String>,
) -> Result<Option<TlsCertificates>, LiveClientError> {
    let root_cert = match ca {
        Some(path) => Some(fs::read(path).map_err(|_source| LiveClientError::TlsFile {
            key: "ssl_ca_certs",
        })?),
        None => None,
    };
    let client_tls = match (cert, key) {
        (Some(cert_path), Some(key_path)) => Some(ClientTlsConfig {
            client_cert: fs::read(cert_path).map_err(|_source| LiveClientError::TlsFile {
                key: "ssl_certfile",
            })?,
            client_key: fs::read(key_path)
                .map_err(|_source| LiveClientError::TlsFile { key: "ssl_keyfile" })?,
        }),
        (None, None) => None,
        (Some(_), None) => {
            return Err(LiveClientError::InvalidSetting {
                field: "ssl_keyfile",
            });
        }
        (None, Some(_)) => {
            return Err(LiveClientError::InvalidSetting {
                field: "ssl_certfile",
            });
        }
    };
    if root_cert.is_none() && client_tls.is_none() {
        return Ok(None);
    }
    Ok(Some(TlsCertificates {
        client_tls,
        root_cert,
    }))
}

pub(crate) fn client(
    info: ConnectionInfo,
    settings: &TlsSettings,
) -> Result<Client, LiveClientError> {
    let enabled = settings.state == TlsState::Enabled || settings.certificates.is_some();
    let address_info = if enabled {
        tls_address(info, settings.insecure)
    } else {
        info
    };
    let Some(certificates) = settings.certificates.clone() else {
        return check_hostname(Client::open(address_info)?, enabled, settings);
    };
    check_hostname(
        Client::build_with_tls(address_info, certificates)?,
        enabled,
        settings,
    )
}

fn check_hostname(
    client: Client,
    enabled: bool,
    settings: &TlsSettings,
) -> Result<Client, LiveClientError> {
    if !enabled || !settings.accept_invalid_hostnames {
        return Ok(client);
    }
    let info = client.get_connection_info().clone();
    let mut addr = info.addr().clone();
    addr.set_danger_accept_invalid_hostnames(true);
    Ok(Client::open(info.set_addr(addr))?)
}

fn tls_address(info: ConnectionInfo, insecure: bool) -> ConnectionInfo {
    let (host, port) = match info.addr().clone() {
        ConnectionAddr::Tcp(host, port) | ConnectionAddr::TcpTls { host, port, .. } => (host, port),
        ConnectionAddr::Unix(_) | _ => return info,
    };
    info.set_addr(ConnectionAddr::TcpTls {
        host,
        port,
        insecure,
        tls_params: None,
    })
}

pub(crate) fn configure_cluster(
    mut builder: ClusterClientBuilder,
    settings: &TlsSettings,
) -> ClusterClientBuilder {
    if settings.state != TlsState::Enabled && settings.certificates.is_none() {
        return builder;
    }
    builder = builder.tls(if settings.insecure {
        TlsMode::Insecure
    } else {
        TlsMode::Secure
    });
    if settings.accept_invalid_hostnames {
        builder = builder.danger_accept_invalid_hostnames(true);
    }
    let Some(certificates) = settings.certificates.clone() else {
        return builder;
    };
    builder.certs(certificates)
}

pub(crate) fn warn_unmapped(map: &BTreeMap<String, PassthroughValue>, extra: &[&str]) {
    for key in map.keys() {
        if TLS_KEYS.contains(&key.as_str()) || extra.contains(&key.as_str()) {
            continue;
        }
        tracing::warn!(key = %key, "redis: the connection option is not supported and has no effect");
    }
}
