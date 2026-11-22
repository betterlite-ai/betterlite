use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::Serialize;
use url::form_urlencoded;

use super::SamlIdpError;
use super::authn_request::decode_authn_request_id;
use super::runner::run_fixture_script;
use crate::case::saml_assertion::{Mutation, SamlAssertion};
use betterlite_oracle_runner::case::RequestBody;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");
const FIXTURE_DIRECTORY: &str = "saml-idp-fixture";
const DRIVER_SCRIPT: &str = "driver.py";
const KEYPAIR_SCRIPT: &str = "keypair.py";
const FORM_CONTENT_TYPE: &str = "application/x-www-form-urlencoded";
const IDP_ENTITY: &str = "urn:betterlite:oracle:idp";
const IDP_SSO_URL: &str = "http://idp.invalid/sso";
// python3-saml allows 300 seconds of clock drift on Conditions, so the expiry must clear that window.
const EXPIRED_NOT_BEFORE_SECONDS: i64 = -960;
const EXPIRED_NOT_ON_OR_AFTER_SECONDS: i64 = -360;
const TIME_FORMAT: &str = "%Y-%m-%dT%H:%M:%SZ";

#[derive(Serialize)]
struct SigningInput<'input> {
    key_directory: &'input str,
    issuer: &'input str,
    subject: &'input str,
    attributes: &'input BTreeMap<String, Vec<String>>,
    audience: &'input str,
    destination: &'input str,
    in_response_to: &'input str,
    assertion_id: &'input str,
    issue_instant: &'input str,
    not_before: &'input str,
    not_on_or_after: &'input str,
    mutation: Option<Mutation>,
}

#[derive(Serialize)]
struct KeypairInput<'input> {
    entity_id: &'input str,
    sso_url: &'input str,
    output_directory: &'input str,
}

pub(crate) struct SamlSigner {
    pub(crate) directory: PathBuf,
    signed_at: DateTime<Utc>,
    cache: BTreeMap<Vec<u8>, String>,
}

impl SamlSigner {
    pub(crate) fn new() -> Self {
        Self {
            directory: Path::new(ROOT).join("../../target").join(FIXTURE_DIRECTORY),
            signed_at: Utc::now(),
            cache: BTreeMap::new(),
        }
    }

    pub(crate) fn form_body(
        &mut self,
        assertion: &SamlAssertion,
        request_value: &str,
    ) -> Result<RequestBody, SamlIdpError> {
        let in_response_to = decode_authn_request_id(request_value.as_bytes())?;
        let document = self.document(assertion, &in_response_to)?;
        let cached = self.cache.get(&document).cloned();
        let response = match cached {
            Some(cached) => cached,
            None => {
                ensure_keypair(&self.directory)?;
                let output = run_fixture_script(DRIVER_SCRIPT, &document)?;
                let text = String::from_utf8(output).map_err(SamlIdpError::Output)?;
                STANDARD
                    .decode(text.as_bytes())
                    .map_err(SamlIdpError::ResponseBase64)?;
                self.cache.insert(document, text.clone());
                text
            }
        };
        Ok(RequestBody::Raw {
            content_type: FORM_CONTENT_TYPE.to_owned(),
            bytes: form_bytes(&response, assertion.relay_state.as_deref()),
        })
    }

    fn document(
        &self,
        assertion: &SamlAssertion,
        in_response_to: &str,
    ) -> Result<Vec<u8>, SamlIdpError> {
        let (not_before, not_on_or_after) = validity(assertion, self.signed_at)?;
        let one_second = ChronoDuration::try_seconds(1).ok_or(SamlIdpError::TimeRange)?;
        let issued_at = self
            .signed_at
            .checked_sub_signed(one_second)
            .ok_or(SamlIdpError::TimeRange)?;
        let issue_instant = issued_at.format(TIME_FORMAT).to_string();
        let not_before = not_before.format(TIME_FORMAT).to_string();
        let not_on_or_after = not_on_or_after.format(TIME_FORMAT).to_string();
        let key_directory = self.directory.to_str().ok_or(SamlIdpError::FixturePath)?;
        let input = SigningInput {
            key_directory,
            issuer: IDP_ENTITY,
            subject: &assertion.subject,
            attributes: &assertion.attributes,
            audience: &assertion.audience,
            destination: &assertion.destination,
            in_response_to,
            assertion_id: &assertion.assertion_id,
            issue_instant: &issue_instant,
            not_before: &not_before,
            not_on_or_after: &not_on_or_after,
            mutation: assertion.mutation,
        };
        serde_json::to_vec(&input).map_err(SamlIdpError::Json)
    }
}

fn form_bytes(response: &str, relay_state: Option<&str>) -> Vec<u8> {
    let mut form = form_urlencoded::Serializer::new(String::new());
    form.append_pair("SAMLResponse", response);
    if let Some(relay_state) = relay_state {
        form.append_pair("RelayState", relay_state);
    }
    form.finish().into_bytes()
}

fn validity(
    assertion: &SamlAssertion,
    signed_at: DateTime<Utc>,
) -> Result<(DateTime<Utc>, DateTime<Utc>), SamlIdpError> {
    let (before, after) = if assertion.mutation == Some(Mutation::Expired) {
        (EXPIRED_NOT_BEFORE_SECONDS, EXPIRED_NOT_ON_OR_AFTER_SECONDS)
    } else {
        (
            assertion.not_before_offset_seconds,
            assertion.not_on_or_after_offset_seconds,
        )
    };
    let not_before = offset_time(signed_at, before)?;
    let not_on_or_after = offset_time(signed_at, after)?;
    Ok((not_before, not_on_or_after))
}

fn offset_time(
    signed_at: DateTime<Utc>,
    offset_seconds: i64,
) -> Result<DateTime<Utc>, SamlIdpError> {
    let offset = ChronoDuration::try_seconds(offset_seconds).ok_or(SamlIdpError::TimeRange)?;
    signed_at
        .checked_add_signed(offset)
        .ok_or(SamlIdpError::TimeRange)
}

pub(crate) fn ensure_keypair(directory: &Path) -> Result<(), SamlIdpError> {
    if keypair_present(directory) {
        return Ok(());
    }
    let parent = directory
        .parent()
        .ok_or_else(|| SamlIdpError::Fixture(directory.to_owned()))?;
    fs::create_dir_all(parent).map_err(SamlIdpError::Io)?;
    let staging = tempfile::Builder::new()
        .prefix("saml-idp-keypair-")
        .tempdir_in(parent)
        .map_err(SamlIdpError::Io)?;
    let output_directory = staging.path().to_str().ok_or(SamlIdpError::FixturePath)?;
    let input = KeypairInput {
        entity_id: IDP_ENTITY,
        sso_url: IDP_SSO_URL,
        output_directory,
    };
    let document = serde_json::to_vec(&input).map_err(SamlIdpError::Json)?;
    run_fixture_script(KEYPAIR_SCRIPT, &document)?;
    match fs::rename(staging.path(), directory) {
        Ok(()) => Ok(()),
        Err(_) if keypair_present(directory) => Ok(()),
        Err(source) => Err(SamlIdpError::Io(source)),
    }
}

fn keypair_present(directory: &Path) -> bool {
    let key = directory.join("key.pem");
    let certificate = directory.join("cert.pem");
    key.is_file() && certificate.is_file()
}
