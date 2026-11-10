use url::Url;

use betterlite_config::sections::azure_client_secret::Settings;

use super::ClientSecretError;
use super::transport::{HttpMethod, HttpRequest, Transport};

pub const IMDS_REGION_URL: &str =
    "http://169.254.169.254/metadata/instance/compute/location?format=text&api-version=2021-01-01";
pub const PUBLIC_CLOUD: &str = "login.microsoftonline.com";

const CIAM_SUFFIX: &str = ".ciamlogin.com";
const WELL_KNOWN_AUTHORITY_HOSTS: [&str; 13] = [
    PUBLIC_CLOUD,
    "login.microsoft.com",
    "login.windows.net",
    "sts.windows.net",
    "login.chinacloudapi.cn",
    "login.partner.microsoftonline.cn",
    "login.microsoftonline.de",
    "login-us.microsoftonline.com",
    "login.microsoftonline.us",
    "login.usgovcloudapi.net",
    "login.sovcloud-identity.fr",
    "login.sovcloud-identity.de",
    "login.sovcloud-identity.sg",
];
const WELL_KNOWN_B2C_HOSTS: [&str; 5] = [
    "b2clogin.com",
    "b2clogin.cn",
    "b2clogin.us",
    "b2clogin.de",
    "ciamlogin.com",
];
const CENTRAL_ALIASES: [&str; 4] = [
    PUBLIC_CLOUD,
    "login.microsoft.com",
    "login.windows.net",
    "sts.windows.net",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Authority {
    pub instance: String,
    pub tenant: String,
    pub token_endpoint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfiguredRegion {
    AutoDetect,
    Named(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RegionSelection {
    pub configured: Option<ConfiguredRegion>,
    pub detected: Option<String>,
    pub region: Option<String>,
    pub opted_in: bool,
    pub warnings: Vec<String>,
    pub region_debug: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flavour {
    Entra,
    Ciam,
    Adfs,
    B2c,
}

struct Canonical {
    url: Url,
    instance: String,
    tenant: String,
    path: String,
    flavour: Flavour,
}

#[derive(serde::Deserialize)]
struct InstancePayload {
    error: Option<String>,
    tenant_discovery_endpoint: Option<String>,
}

#[derive(serde::Deserialize)]
struct OpenIdConfiguration {
    authorization_endpoint: Option<String>,
    token_endpoint: Option<String>,
}

pub fn normalize_authority(authority: &str) -> Result<String, ClientSecretError> {
    let trimmed = authority.trim_end_matches([' ', '/']);
    match Url::parse(authority) {
        Ok(parsed) if parsed.scheme() == "https" => Ok(trimmed.to_owned()),
        Ok(_parsed) => Err(invalid_authority(authority)),
        Err(url::ParseError::RelativeUrlWithoutBase) => Ok(format!("https://{trimmed}")),
        Err(_error) => Err(invalid_authority(authority)),
    }
}

pub fn discover_authority(
    transport: &dyn Transport,
    authority_host: &str,
    tenant_id: &str,
    instance_discovery: bool,
) -> Result<Authority, ClientSecretError> {
    let normalized = normalize_authority(authority_host)?;
    let address = format!("{normalized}/{tenant_id}");
    let canonical = canonicalize(&address)?;
    let endpoint = if instance_discovery && canonical.requires_instance_discovery() {
        instance_discovery_endpoint(transport, &canonical)?
    } else {
        canonical.tenant_discovery_endpoint()
    };
    tenant_discovery(transport, &endpoint, &canonical)
}

pub fn regional_authority(
    transport: &dyn Transport,
    central: &Authority,
    region: &str,
) -> Result<Authority, ClientSecretError> {
    let host = if CENTRAL_ALIASES.contains(&central.instance.as_str()) {
        format!("{region}.login.microsoft.com")
    } else {
        format!("{region}.{}", central.instance)
    };
    discover_authority(
        transport,
        &format!("https://{host}"),
        &central.tenant,
        false,
    )
}

pub fn detect_region_of_azure_vm(transport: &dyn Transport) -> Option<String> {
    let request = HttpRequest {
        method: HttpMethod::Get,
        url: IMDS_REGION_URL.to_owned(),
        query: Vec::new(),
        headers: vec![("Metadata".to_owned(), "true".to_owned())],
        form: Vec::new(),
    };
    transport.send(&request).map_or_else(
        |_error| {
            tracing::info!("IMDS {IMDS_REGION_URL} unavailable. Perhaps not running in Azure VM?");
            None
        },
        |reply| Some(reply.body.trim().to_owned()),
    )
}

pub fn select_region(settings: &Settings, transport: &dyn Transport) -> RegionSelection {
    let raw = settings.azure_regional_authority_name.as_deref();
    let forced = settings
        .msal_force_region
        .as_deref()
        .filter(|value| !value.is_empty());
    let configured = raw
        .map(configured_region)
        .or_else(|| forced.map(|region| ConfiguredRegion::Named(region.to_owned())));
    let opted_in = raw.map_or_else(|| forced.is_some(), |value| !value.is_empty());
    let detected = detected_region(&settings.region_name, configured.as_ref(), transport);
    let warnings = region_warning(configured.as_ref(), detected.as_deref())
        .into_iter()
        .collect::<Vec<String>>();
    for message in &warnings {
        tracing::warn!("{message}");
    }
    let region = region_to_use(configured.as_ref(), detected.as_deref());
    let region_debug = format!("Region to be used: {}", region_value(region.as_deref()));
    tracing::debug!("{region_debug}");
    RegionSelection {
        configured,
        detected,
        region,
        opted_in,
        warnings,
        region_debug,
    }
}

fn detected_region(
    region_name: &str,
    configured: Option<&ConfiguredRegion>,
    transport: &dyn Transport,
) -> Option<String> {
    let local = region_name.replace(' ', "").to_lowercase();
    if !local.is_empty() {
        return Some(local);
    }
    if configured.is_some() {
        detect_region_of_azure_vm(transport)
    } else {
        None
    }
}

fn region_to_use(configured: Option<&ConfiguredRegion>, detected: Option<&str>) -> Option<String> {
    match configured {
        Some(ConfiguredRegion::AutoDetect) => detected.map(str::to_owned),
        Some(ConfiguredRegion::Named(name)) => Some(name.clone()),
        None => None,
    }
}

#[must_use]
pub fn configured_region(value: &str) -> ConfiguredRegion {
    // azure-identity maps only these two words to auto detection; every other string, including an empty one, is a literal region
    let lowered = value.to_ascii_lowercase();
    let auto = lowered == "true" || lowered == "tryautodetect";
    if !value.is_empty() && auto {
        ConfiguredRegion::AutoDetect
    } else {
        ConfiguredRegion::Named(value.to_owned())
    }
}

fn region_warning(configured: Option<&ConfiguredRegion>, detected: Option<&str>) -> Option<String> {
    match configured {
        None => detected.is_some().then(|| {
            format!(
                "Region configured (None) != region detected ({})",
                region_value(detected)
            )
        }),
        Some(ConfiguredRegion::AutoDetect) => None,
        Some(ConfiguredRegion::Named(name)) => (Some(name.as_str()) != detected).then(|| {
            format!(
                "Region configured ({}) != region detected ({})",
                region_value(Some(name)),
                region_value(detected)
            )
        }),
    }
}

fn region_value(value: Option<&str>) -> String {
    value.map_or_else(
        || "None".to_owned(),
        |text| {
            let escaped = text.replace('\\', "\\\\").replace('\'', "\\'");
            format!("'{escaped}'")
        },
    )
}

fn canonicalize(address: &str) -> Result<Canonical, ClientSecretError> {
    let url = Url::parse(address).map_err(|_error| invalid_authority_address(address))?;
    if url.scheme() != "https" {
        return Err(invalid_authority_address(address));
    }
    let Some(instance) = url.host_str().map(str::to_owned) else {
        return Err(invalid_authority_address(address));
    };
    let path = url.path().to_owned();
    let segments: Vec<&str> = path.split('/').collect();
    let first = segments
        .get(1)
        .copied()
        .filter(|segment| !segment.is_empty());
    let is_ciam = instance.ends_with(CIAM_SUFFIX);
    let tenant = if is_ciam {
        first.map_or_else(
            || format!("{}.onmicrosoft.com", instance.trim_end_matches(CIAM_SUFFIX)),
            str::to_owned,
        )
    } else {
        first
            .map(str::to_owned)
            .ok_or_else(|| invalid_authority_address(address))?
    };
    let flavour = flavour(&instance, &tenant, &segments, is_ciam);
    Ok(Canonical {
        url,
        instance,
        tenant,
        path,
        flavour,
    })
}

fn flavour(instance: &str, tenant: &str, segments: &[&str], is_ciam: bool) -> Flavour {
    if is_ciam {
        return Flavour::Ciam;
    }
    if tenant.eq_ignore_ascii_case("adfs") {
        return Flavour::Adfs;
    }
    let b2c_host = WELL_KNOWN_B2C_HOSTS
        .iter()
        .any(|suffix| instance.ends_with(&format!(".{suffix}")));
    let b2c_path = segments.len() == 3
        && segments
            .get(2)
            .is_some_and(|part| part.to_lowercase().starts_with("b2c_"));
    if b2c_host || b2c_path {
        Flavour::B2c
    } else {
        Flavour::Entra
    }
}

impl Canonical {
    fn requires_instance_discovery(&self) -> bool {
        let known = WELL_KNOWN_AUTHORITY_HOSTS.contains(&self.instance.as_str());
        let known_to_developer =
            matches!(self.flavour, Flavour::Ciam | Flavour::Adfs | Flavour::B2c);
        !(known || known_to_developer)
    }

    fn tenant_discovery_endpoint(&self) -> String {
        let prefix = if self.flavour == Flavour::Ciam && self.path.len() <= 1 {
            self.tenant.clone()
        } else {
            self.path.clone()
        };
        let version = if self.flavour == Flavour::Adfs {
            ""
        } else {
            "/v2.0"
        };
        let mut url = self.url.clone();
        url.set_path(&format!(
            "{prefix}{version}/.well-known/openid-configuration"
        ));
        url.to_string()
    }
}

fn instance_discovery_endpoint(
    transport: &dyn Transport,
    canonical: &Canonical,
) -> Result<String, ClientSecretError> {
    let host = if WELL_KNOWN_AUTHORITY_HOSTS.contains(&canonical.instance.as_str()) {
        canonical.instance.clone()
    } else {
        PUBLIC_CLOUD.to_owned()
    };
    let authorization_endpoint = format!(
        "https://{}{}/oauth2/v2.0/authorize",
        canonical.instance, canonical.path
    );
    let request = HttpRequest {
        method: HttpMethod::Get,
        url: format!("https://{host}/common/discovery/instance"),
        query: vec![
            ("authorization_endpoint".to_owned(), authorization_endpoint),
            ("api-version".to_owned(), "1.0".to_owned()),
        ],
        headers: Vec::new(),
        form: Vec::new(),
    };
    let reply = transport
        .send(&request)
        .map_err(|source| ClientSecretError::AuthorityTransport { source })?;
    let payload: InstancePayload = serde_json::from_str(&reply.body)
        .map_err(|source| ClientSecretError::AuthorityResponse { source })?;
    if payload.error.as_deref() == Some("invalid_instance") {
        return Err(ClientSecretError::InvalidInstance {
            authority: canonical.url.to_string(),
        });
    }
    payload
        .tenant_discovery_endpoint
        .ok_or(ClientSecretError::InstanceMetadata)
}

fn tenant_discovery(
    transport: &dyn Transport,
    endpoint: &str,
    canonical: &Canonical,
) -> Result<Authority, ClientSecretError> {
    let request = HttpRequest {
        method: HttpMethod::Get,
        url: endpoint.to_owned(),
        query: Vec::new(),
        headers: Vec::new(),
        form: Vec::new(),
    };
    let reply = transport
        .send(&request)
        .map_err(|source| ClientSecretError::AuthorityTransport { source })?;
    if reply.status != 200 {
        return Err(discovery_status_error(reply.status, canonical));
    }
    let configuration: OpenIdConfiguration =
        serde_json::from_str(&reply.body).map_err(|_error| {
            ClientSecretError::AuthorityConfiguration {
                authority: canonical.url.to_string(),
            }
        })?;
    let _authorization_endpoint =
        configuration
            .authorization_endpoint
            .ok_or(ClientSecretError::AuthorityMissingField {
                field: "authorization_endpoint",
            })?;
    let token_endpoint =
        configuration
            .token_endpoint
            .ok_or(ClientSecretError::AuthorityMissingField {
                field: "token_endpoint",
            })?;
    let endpoint_authority = canonicalize(&token_endpoint)?;
    Ok(Authority {
        instance: canonical.instance.clone(),
        tenant: endpoint_authority.tenant,
        token_endpoint,
    })
}

fn discovery_status_error(status: u16, canonical: &Canonical) -> ClientSecretError {
    if (400..500).contains(&status) {
        ClientSecretError::AuthorityConfiguration {
            authority: canonical.url.to_string(),
        }
    } else {
        ClientSecretError::AuthorityStatus { status }
    }
}

fn invalid_authority(authority: &str) -> ClientSecretError {
    ClientSecretError::InvalidAuthority {
        authority: authority.to_owned(),
    }
}

fn invalid_authority_address(address: &str) -> ClientSecretError {
    ClientSecretError::AuthorityAddress {
        authority: address.to_owned(),
    }
}
