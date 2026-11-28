use betterlite_config::saml_provider::{DEFAULT_NAME_ID_FORMAT, SamlFlag, SamlProviderSettings};
use regex::Regex;
use thiserror::Error;

use super::idp_metadata::IdpSettings;

pub const WANT_NAME_ID: bool = true;
pub const REQUESTED_AUTHN_CONTEXT: bool = false;
pub const HTTP_POST_BINDING: &str = "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST";

// Django's URL regex as python3-saml vendors it; the scheme is validated separately.
const URL_PATTERN: &str = r"^(?:[a-z0-9\.\-]*)://(?:(?:[A-Z0-9_](?:[A-Z0-9-_]{0,61}[A-Z0-9_])?\.)+(?:[A-Z]{2,6}\.?|[A-Z0-9-]{2,}\.?)|localhost|\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}|\[?[A-F0-9]*:[A-F0-9:]+\]?)(?::\d+)?(?:/?|[/?]\S+)$";
const URL_SCHEMES: [&str; 4] = ["http", "https", "ftp", "ftps"];

#[must_use]
pub fn validate_url(url: &str) -> bool {
    let scheme = url.split("://").next().unwrap_or(url);
    if !URL_SCHEMES
        .iter()
        .any(|known| scheme.eq_ignore_ascii_case(known))
    {
        return false;
    }
    Regex::new(&format!("(?i){URL_PATTERN}")).is_ok_and(|pattern| pattern.is_match(url))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergedSettings {
    pub strict: SamlFlag,
    pub sp_entity_id: String,
    pub acs_url: String,
    pub name_id_format: String,
    pub want_assertions_signed: SamlFlag,
    pub want_messages_signed: SamlFlag,
    pub authn_requests_signed: SamlFlag,
    pub idp: IdpSettings,
}

#[must_use]
pub fn merged_settings(
    settings: &SamlProviderSettings,
    idp: &IdpSettings,
    metadata_url: &str,
    acs_url: &str,
) -> MergedSettings {
    let sp_entity_id = settings
        .sp_entity_id
        .clone()
        .unwrap_or_else(|| metadata_url.to_owned());
    let name_id_format = idp
        .name_id_format
        .clone()
        .or_else(|| settings.sp_name_id_format.clone())
        .unwrap_or_else(|| DEFAULT_NAME_ID_FORMAT.to_owned());
    MergedSettings {
        strict: settings.strict,
        sp_entity_id,
        acs_url: acs_url.to_owned(),
        name_id_format,
        want_assertions_signed: settings.want_assertions_signed,
        want_messages_signed: settings.want_messages_signed,
        authn_requests_signed: SamlFlag(
            idp.want_authn_requests_signed
                .unwrap_or(settings.authn_requests_signed.0),
        ),
        idp: idp.clone(),
    }
}

impl MergedSettings {
    pub fn validate(&self, sp_validation_only: bool) -> Result<(), SettingsErrors> {
        let mut codes = Vec::new();
        if !sp_validation_only {
            self.check_idp(&mut codes);
        }
        self.check_sp(&mut codes);
        if codes.is_empty() {
            Ok(())
        } else {
            Err(SettingsErrors { codes })
        }
    }

    fn check_idp(&self, codes: &mut Vec<&'static str>) {
        if self.idp.entity_id.as_deref().unwrap_or_default().is_empty() {
            codes.push("idp_entityId_not_found");
        }
        let sso_url = self.idp.sso_url.as_deref().unwrap_or_default();
        if sso_url.is_empty() {
            codes.push("idp_sso_not_found");
        } else if !validate_url(sso_url) {
            codes.push("idp_sso_url_invalid");
        }
        if let Some(slo_url) = self.idp.slo_url.as_deref()
            && !slo_url.is_empty()
            && !validate_url(slo_url)
        {
            codes.push("idp_slo_url_invalid");
        }
        let has_certificate = self
            .idp
            .x509_cert
            .as_deref()
            .is_some_and(|value| !value.is_empty())
            || self
                .idp
                .x509_cert_multi
                .as_ref()
                .and_then(|multi| multi.signing.as_ref())
                .is_some_and(|signing| !signing.is_empty());
        if (self.want_assertions_signed.0 || self.want_messages_signed.0) && !has_certificate {
            codes.push("idp_cert_or_fingerprint_not_found_and_required");
        }
    }

    fn check_sp(&self, codes: &mut Vec<&'static str>) {
        if self.sp_entity_id.is_empty() {
            codes.push("sp_entityId_not_found");
        }
        if self.acs_url.is_empty() {
            codes.push("sp_acs_not_found");
        } else if !validate_url(&self.acs_url) {
            codes.push("sp_acs_url_invalid");
        }
        // python3-saml ships no SP key or certificate, so requesting signed requests always fails its check.
        if self.authn_requests_signed.0 {
            codes.push("sp_cert_not_found_and_required");
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("Invalid dict settings: {}", .codes.join(","))]
pub struct SettingsErrors {
    pub codes: Vec<&'static str>,
}
