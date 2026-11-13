use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use secrecy::{ExposeSecret as _, SecretSlice, SecretString};

use super::fixture::{
    CERTIFICATE_ONLY, CLIENT, CountingIds, EC_P12, EC_PEM, Fixture, KEYLESS_P12, LEAF_SHA1,
    LEAF_X5T, NOW, PASSWORD, RSA_CHAIN, RSA_ENCRYPTED, RSA_LEGACY_P12, RSA_NO_CERTIFICATE, RSA_P12,
    RSA_PKCS1, RSA_PKCS8, RSA_UNPROTECTED_P12, TENANT, TOKEN_URL, decode, form, input, openid,
    token_reply,
};
use crate::azure_certificate::CertificateError;
use crate::azure_certificate::assertion::Assertions;
use crate::azure_certificate::credential::CertificateInput;
use crate::azure_certificate::material::{CertificateMaterial, decode_certificate_material};
use crate::azure_client_secret::DEFAULT_SCOPE;
use crate::azure_client_secret::token::AccessToken;

const DESERIALIZE: &str = "Failed to deserialize certificate in PEM or PKCS12 format";
const INVALID_TENANT: &str = "Invalid tenant ID provided. You can locate your tenant ID by following the instructions here: https://learn.microsoft.com/partner-center/find-ids-and-domain-names";
const PEM_FRAMING: &str = "Unable to load PEM file. See https://cryptography.io/en/latest/faq/#why-can-t-i-import-my-pem-file for more details. MalformedFraming";
const KEY_DATA: &str = "Could not deserialize key data. The data may be in an incorrect format, it may be encrypted with an unsupported algorithm, or it may be an unsupported key type (e.g. EC curves with explicit parameters).";
const LEGACY_ENCRYPTED: &str =
    "a PEM private key encrypted with a Proc-Type header is not supported";
const RSA_ONLY: &str =
    "The certificate must have an RSA private key because RS256 is used for signing";
const SHORT_TOKEN: &str = r#"{"access_token":"t","expires_in":60}"#;

fn load(data: &[u8], password: Option<&str>) -> Result<CertificateMaterial, CertificateError> {
    let secret = password.map(SecretString::from);
    decode_certificate_material(data, secret.as_ref())
}

fn failure(data: &[u8], password: Option<&str>) -> String {
    load(data, password).unwrap_err().to_string()
}

fn signed_with(material: CertificateMaterial) -> String {
    let assertions = Assertions {
        material,
        client_id: CLIENT.to_owned(),
        send_chain: false,
        ids: Arc::new(CountingIds(AtomicUsize::new(1))),
        reusable: Mutex::new(None),
    };
    let assertion = assertions.assertion(TOKEN_URL, NOW).unwrap();
    assertion.expose_secret().to_owned()
}

fn path_input(path: &str) -> CertificateInput {
    let mut by_path = input(b"");
    by_path.certificate_data = None;
    by_path.certificate_path = Some(path.to_owned());
    by_path
}

pub(super) fn pem_certificate() {
    for (data, chain) in [(RSA_PKCS8, 1), (RSA_PKCS1, 1), (RSA_CHAIN, 2)] {
        let material = load(data, None).unwrap();
        assert_eq!(hex::encode(material.thumbprint), LEAF_SHA1);
        assert_eq!(material.chain.len(), chain);
        let (header, claims) = decode(&signed_with(material));
        assert_eq!(header.x5t.as_deref(), Some(LEAF_X5T));
        assert_eq!(claims.iss, CLIENT);
    }
    let encrypted = load(RSA_ENCRYPTED, Some(PASSWORD)).unwrap();
    assert_eq!(hex::encode(encrypted.thumbprint), LEAF_SHA1);
    // an empty password counts as none
    let _plain = load(RSA_PKCS8, Some("")).unwrap();
    let mut fixture = Fixture::new(Vec::new());
    fixture
        .files
        .files
        .insert("/certs/leaf.pem".to_owned(), RSA_PKCS8.to_vec());
    fixture.credential(path_input("/certs/leaf.pem")).unwrap();
    assert_eq!(*fixture.files.reads.lock().unwrap(), ["/certs/leaf.pem"]);
}

pub(super) fn pkcs12_password() {
    for (data, password) in [
        (RSA_P12, Some(PASSWORD)),
        (RSA_LEGACY_P12, Some(PASSWORD)),
        (RSA_UNPROTECTED_P12, None),
    ] {
        let material = load(data, password).unwrap();
        assert_eq!(hex::encode(material.thumbprint), LEAF_SHA1);
        decode(&signed_with(material));
    }
    // the signing certificate leads the chain, the CA follows
    assert_eq!(load(RSA_P12, Some(PASSWORD)).unwrap().chain.len(), 2);
    assert_eq!(failure(RSA_P12, None), DESERIALIZE);
    assert_eq!(failure(RSA_P12, Some("wrong")), DESERIALIZE);
    assert_eq!(failure(b"not a certificate", None), DESERIALIZE);
    assert_eq!(
        failure(KEYLESS_P12, Some(PASSWORD)),
        "The certificate must include its private key"
    );
}

// the first token expires after `lifetime`, so the second request is made at `at`
fn assertions_after(lifetime: i64, at: i64) -> (String, String) {
    let first = format!(r#"{{"access_token":"a","expires_in":{lifetime}}}"#);
    let second = token_reply(r#"{"access_token":"b","expires_in":3600}"#);
    let fixture = Fixture::new(vec![openid(TENANT), token_reply(&first), second]);
    let credential = fixture.credential(input(RSA_PKCS8)).unwrap();
    credential.token().unwrap();
    fixture.clock.0.store(at, Ordering::SeqCst);
    credential.token().unwrap();
    let posts = fixture.transport.posts();
    assert_eq!(posts.len(), 2);
    (
        form(&posts[0])["client_assertion"].clone(),
        form(&posts[1])["client_assertion"].clone(),
    )
}

pub(super) fn client_assertion_claims() {
    let (reused, again) = assertions_after(539, NOW + 539);
    assert_eq!(reused, again);
    let (first, second) = assertions_after(540, NOW + 540);
    assert_ne!(first, second);
    let (header, claims) = decode(&first);
    assert_eq!(header.alg, jsonwebtoken::Algorithm::RS256);
    assert_eq!(header.typ.as_deref(), Some("JWT"));
    assert_eq!(header.x5t.as_deref(), Some(LEAF_X5T));
    assert!(header.x5c.is_none());
    assert_eq!(claims.aud, TOKEN_URL);
    assert_eq!((claims.iss.as_str(), claims.sub.as_str()), (CLIENT, CLIENT));
    assert_eq!((claims.iat, claims.exp), (NOW, NOW + 600));
    assert_eq!(claims.jti, "jti-1");
    assert_eq!(claims.nbf, None);
    let (_, regenerated) = decode(&second);
    assert_eq!(regenerated.jti, "jti-2");
    assert_eq!((regenerated.iat, regenerated.exp), (NOW + 540, NOW + 1140));
}

pub(super) fn invalid_certificate() {
    assert_eq!(failure(EC_PEM, None), RSA_ONLY);
    assert_eq!(failure(EC_P12, Some(PASSWORD)), RSA_ONLY);
    assert_eq!(
        failure(CERTIFICATE_ONLY, None),
        "Valid PEM but no BEGIN/END delimiters for a private key found. Are you sure this is a private key?"
    );
    assert_eq!(
        failure(RSA_NO_CERTIFICATE, None),
        "Valid PEM but no BEGIN CERTIFICATE/END CERTIFICATE delimiters. Are you sure this is a certificate?"
    );
    assert_eq!(
        failure(RSA_ENCRYPTED, None),
        "Password was not given but private key is encrypted"
    );
    assert_eq!(
        failure(RSA_ENCRYPTED, Some("wrong")),
        "Incorrect password, could not decrypt key"
    );
    assert_eq!(
        failure(RSA_PKCS8, Some(PASSWORD)),
        "Password was given but private key is not encrypted."
    );
    assert_eq!(failure(&RSA_PKCS8[..400], None), PEM_FRAMING);
    assert_eq!(failure(b"-----BEGIN junk", None), PEM_FRAMING);
    unreadable_keys();
    non_rsa_pkcs1_keys();
    unusable_sources();
}

fn non_rsa_pkcs1_keys() {
    let certificate = str::from_utf8(CERTIFICATE_ONLY).unwrap();
    let ec = format!(
        "-----BEGIN EC PRIVATE KEY-----\nAAAA\n-----END EC PRIVATE KEY-----\n{certificate}"
    );
    let dsa = format!(
        "-----BEGIN DSA PRIVATE KEY-----\nAAAA\n-----END DSA PRIVATE KEY-----\n{certificate}"
    );
    assert_eq!(failure(ec.as_bytes(), None), RSA_ONLY);
    assert_eq!(failure(dsa.as_bytes(), None), RSA_ONLY);
}

fn unreadable_keys() {
    let pkcs8 = b"-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n";
    let encrypted =
        b"-----BEGIN ENCRYPTED PRIVATE KEY-----\nAAAA\n-----END ENCRYPTED PRIVATE KEY-----\n";
    let other = b"-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----\n";
    let legacy = b"-----BEGIN RSA PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\nDEK-Info: AES-128-CBC,00112233445566778899AABBCCDDEEFF\n\nAAAA\n-----END RSA PRIVATE KEY-----\n";
    assert_eq!(failure(pkcs8, None), KEY_DATA);
    assert_eq!(failure(encrypted, Some(PASSWORD)), KEY_DATA);
    assert_eq!(failure(other, None), KEY_DATA);
    assert_eq!(failure(legacy, Some(PASSWORD)), LEGACY_ENCRYPTED);
    assert_eq!(
        failure(legacy, None),
        "Password was not given but private key is encrypted"
    );
}

fn unusable_sources() {
    let fixture = Fixture::new(Vec::new());
    let mut both = input(RSA_PKCS8);
    both.certificate_path = Some("/certs/leaf.pem".to_owned());
    let shown = fixture.credential(both).unwrap_err().to_string();
    assert_eq!(
        shown,
        "Please specify either \"certificate_path\" or \"certificate_data\", not both"
    );
    let neither = "CertificateCredential requires a value for either \"certificate_path\" or \"certificate_data\"";
    for (path, data) in [
        (None, None),
        (Some(""), None),
        (None, Some(&b""[..])),
        (Some(""), Some(&b""[..])),
    ] {
        let mut empty = input(b"");
        empty.certificate_path = path.map(str::to_owned);
        empty.certificate_data = data.map(|bytes| SecretSlice::from(bytes.to_vec()));
        let rejection = fixture.credential(empty).unwrap_err().to_string();
        assert_eq!(rejection, neither);
    }
    let missing = fixture
        .credential(path_input("/certs/missing.pem"))
        .unwrap_err();
    assert!(matches!(missing, CertificateError::Read { .. }));
    assert!(fixture.transport.requests().is_empty());
}

fn acquire(status: u16, body: &str) -> (Fixture, Result<AccessToken, CertificateError>) {
    let fixture = Fixture::new(vec![openid(TENANT), (status, body.to_owned())]);
    let outcome = fixture.credential(input(RSA_PKCS8)).unwrap().token();
    (fixture, outcome)
}

fn rejected(status: u16, body: &str) -> String {
    acquire(status, body).1.unwrap_err().to_string()
}

pub(super) fn token_response() {
    let (fixture, outcome) = acquire(200, r#"{"access_token":"tok","expires_in":3600}"#);
    let issued = outcome.unwrap();
    assert_eq!(issued.token.expose_secret(), "tok");
    assert_eq!((issued.expires_on, issued.refresh_on), (NOW + 3600, None));
    assert_eq!(issued.token_type, "Bearer");
    let requests = fixture.transport.requests();
    assert_eq!(requests.len(), 2);
    let post = &requests[1];
    assert_eq!(post.url, TOKEN_URL);
    assert_eq!(
        post.headers,
        [("Accept".to_owned(), "application/json".to_owned())]
    );
    let fields = form(post);
    let names: Vec<&str> = fields.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        [
            "client_assertion",
            "client_assertion_type",
            "client_id",
            "client_info",
            "grant_type",
            "scope"
        ]
    );
    assert_eq!(
        fields["client_assertion_type"],
        "urn:ietf:params:oauth:client-assertion-type:jwt-bearer"
    );
    assert_eq!(fields["grant_type"], "client_credentials");
    assert_eq!(fields["client_id"], CLIENT);
    assert_eq!(fields["client_info"], "1");
    assert_eq!(fields["scope"], DEFAULT_SCOPE);
    assert_eq!(decode(&fields["client_assertion"]).1.aud, TOKEN_URL);
    token_fields();
    refresh_and_cache();
    failures();
}

fn token_fields() {
    let relative = r#"{"access_token":"t","expires_in":"60","token_type":"pop","refresh_in":30}"#;
    let popped = acquire(200, relative).1.unwrap();
    assert_eq!(
        (popped.expires_on, popped.refresh_on),
        (NOW + 60, Some(NOW + 30))
    );
    assert_eq!(popped.token_type, "pop");
    let absolute = r#"{"access_token":"t","expires_in":60,"refresh_on":1700000500}"#;
    let refreshed = acquire(200, absolute).1.unwrap();
    assert_eq!(refreshed.refresh_on, Some(1_700_000_500));
}

fn failures() {
    let both = r#"{"error":"invalid_client","error_description":"bad assertion"}"#;
    assert_eq!(rejected(400, both), "Authentication failed: bad assertion");
    let only_error = r#"{"error":"invalid_client"}"#;
    assert_eq!(
        rejected(400, only_error),
        "Authentication failed: invalid_client"
    );
    let empty_description = r#"{"error":"e","error_description":""}"#;
    assert_eq!(rejected(400, empty_description), "Authentication failed: e");
    assert_eq!(rejected(400, "{}"), "Authentication failed: None");
    assert_eq!(
        rejected(503, "unavailable"),
        "Authentication failed: HTTP Error: 503"
    );
    assert_eq!(
        rejected(500, "{}"),
        "Authentication failed: HTTP Error: 500"
    );
    assert_eq!(rejected(400, "not json"), "the token response is not JSON");
}

// one discovery for the credential, and a token is reused until its refresh time
fn refresh_and_cache() {
    let first = r#"{"access_token":"a","expires_in":3600,"refresh_in":30}"#;
    let second = r#"{"access_token":"b","expires_in":3600}"#;
    let replies = vec![openid(TENANT), token_reply(first), token_reply(second)];
    let fixture = Fixture::new(replies);
    let credential = fixture.credential(input(RSA_PKCS8)).unwrap();
    assert_eq!(credential.token().unwrap().token.expose_secret(), "a");
    fixture.clock.0.store(NOW + 29, Ordering::SeqCst);
    assert_eq!(credential.token().unwrap().token.expose_secret(), "a");
    assert_eq!(fixture.transport.requests().len(), 2);
    fixture.clock.0.store(NOW + 30, Ordering::SeqCst);
    assert_eq!(credential.token().unwrap().token.expose_secret(), "b");
    assert_eq!(fixture.transport.requests().len(), 3);
    assert_eq!(fixture.transport.posts().len(), 2);
}

pub(super) fn certificate_argument_boundary() {
    for tenant in ["", "bad/tenant", "a b", "ten\u{1f600}ant", "under_score"] {
        let fixture = Fixture::new(Vec::new());
        let mut bad = path_input("/certs/never-read.pem");
        bad.tenant_id = tenant.to_owned();
        let shown = fixture.credential(bad).unwrap_err().to_string();
        assert_eq!(shown, INVALID_TENANT);
        assert!(fixture.files.reads.lock().unwrap().is_empty());
    }
    for tenant in ["my-tenant.onmicrosoft.com", "0123-abc", "A"] {
        let mut good = input(RSA_PKCS8);
        good.tenant_id = tenant.to_owned();
        Fixture::new(Vec::new()).credential(good).unwrap();
    }
    let mut host = input(RSA_PKCS8);
    host.authority_host = Some("http://login.example.com".to_owned());
    let refused = Fixture::new(Vec::new()).credential(host).unwrap_err();
    assert_eq!(
        refused.to_string(),
        "'http://login.example.com' is an invalid authority. The value must be a TLS protected (https) URL."
    );
    let scoped = CertificateInput {
        scope: Some("api://x/.default".to_owned()),
        ..input(RSA_PKCS8)
    };
    let fixture = Fixture::new(vec![openid(TENANT), token_reply(SHORT_TOKEN)]);
    fixture.credential(scoped).unwrap().token().unwrap();
    let posts = fixture.transport.posts();
    assert_eq!(form(&posts[0])["scope"], "api://x/.default");
}

pub(super) fn certificate_chain_header() {
    let blocks = pem::parse_many(RSA_CHAIN).unwrap();
    let certificates: Vec<String> = blocks
        .iter()
        .filter(|block| block.tag() == "CERTIFICATE")
        .map(|block| STANDARD.encode(block.contents()))
        .collect();
    let key = blocks
        .iter()
        .find(|block| block.tag() == "PRIVATE KEY")
        .unwrap();
    let key_text = STANDARD.encode(key.contents());
    let leaf_only = certificates.first().cloned().into_iter().collect();
    for (data, send, expected) in [
        (RSA_CHAIN, true, Some(certificates)),
        (RSA_CHAIN, false, None),
        (RSA_PKCS8, true, Some(leaf_only)),
    ] {
        let fixture = Fixture::new(vec![openid(TENANT), token_reply(SHORT_TOKEN)]);
        let mut chained = input(data);
        chained.send_certificate_chain = send;
        fixture.credential(chained).unwrap().token().unwrap();
        let assertion = form(&fixture.transport.posts()[0])["client_assertion"].clone();
        assert_eq!(decode(&assertion).0.x5c, expected);
        let segment = assertion.split('.').next().unwrap();
        let header = String::from_utf8(URL_SAFE_NO_PAD.decode(segment).unwrap()).unwrap();
        assert!(!header.contains("PRIVATE"));
        assert!(!header.contains(key_text.get(..40).unwrap()));
    }
}
