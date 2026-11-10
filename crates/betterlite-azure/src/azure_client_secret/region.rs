use betterlite_config::sections::azure_client_secret::Settings;
use secrecy::ExposeSecret as _;

use super::transport::{HttpMethod, HttpRequest, Transport};

pub const IMDS_REGION_URL: &str =
    "http://169.254.169.254/metadata/instance/compute/location?format=text&api-version=2021-01-01";

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
        |reply| Some(reply.body.expose_secret().trim().to_owned()),
    )
}

#[must_use]
pub fn configured_region(value: &str) -> ConfiguredRegion {
    // azure-identity maps only these two words to auto detection; everything else, including an empty string, is a literal region
    let lowered = value.to_ascii_lowercase();
    let auto = lowered == "true" || lowered == "tryautodetect";
    if !value.is_empty() && auto {
        ConfiguredRegion::AutoDetect
    } else {
        ConfiguredRegion::Named(value.to_owned())
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
