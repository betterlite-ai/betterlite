#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use flate2::Compression;
use flate2::write::DeflateEncoder;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::form_urlencoded;

use super::SamlIdpError;
use super::authn_request::{MAX_REQUEST_BYTES, decode_authn_request_id};
use super::driver::{SamlSigner, ensure_keypair};
use super::runner::run_fixture_script;
use crate::case::browser_flow::{Capture, CaptureSource, Step, validate_steps};
use crate::case::decode_case;
use crate::case::saml_assertion::{Mutation, SamlAssertion};
use crate::case::{Compare, Request};
use crate::http::Response;
use crate::http::browser_flow::run_case_steps;
use crate::http::browser_flow::tests::{StubAnswer, StubServer, answer};
use crate::http::client;
use betterlite_oracle_runner::case::BodyKind;
use betterlite_oracle_runner::helpers::sync_litellm;

const PROXY_BASE: &str = "http://127.0.0.1:4000";
const SP_ENTITY: &str = "urn:betterlite:sp";

#[derive(Serialize)]
struct ValidationInput<'input> {
    saml_response: &'input str,
    request_id: &'input str,
    audience: &'input str,
    destination: &'input str,
    metadata: &'input str,
}

#[derive(Deserialize)]
struct ValidationOutput {
    valid: bool,
    error: String,
    response_in_response_to: String,
    assertion_id: String,
}

#[derive(Deserialize)]
struct CorpusSeed {
    saml_request: String,
    request_id: String,
}

#[test]
fn binds_each_signed_response_to_its_proxys_own_authn_request() {
    let metadata = prepare();
    let idp = StubServer::start(vec![answer(200, Vec::new(), &metadata)]);
    let published = client()
        .expect("the client builds")
        .get(format!("{}/idp/metadata", idp.base_url))
        .send()
        .expect("the metadata endpoint answers")
        .text()
        .expect("the metadata body reads");
    assert_eq!(published, metadata);
    let first_seed = corpus_seed("valid-authn-request-a.json");
    let second_seed = corpus_seed("valid-authn-request-b.json");
    let stub = StubServer::start(vec![
        redirect(&first_seed.saml_request),
        answer(200, Vec::new(), ""),
        redirect(&second_seed.saml_request),
        answer(200, Vec::new(), ""),
    ]);
    let steps = [
        redirect_step("authn_a"),
        saml_step(assertion(None, "_assertion_proxy_a"), "authn_a"),
        redirect_step("authn_b"),
        saml_step(assertion(None, "_assertion_proxy_b"), "authn_b"),
    ];
    let responses = run_transcript(&stub, &steps);
    assert_eq!(responses.len(), 4);
    let first_body = stub.received_body(1).expect("the first post arrives");
    let second_body = stub.received_body(3).expect("the second post arrives");
    let (first, first_response) = validate_body(&first_body, &first_seed.request_id, &published);
    let (second, second_response) =
        validate_body(&second_body, &second_seed.request_id, &published);
    assert_bound(&first, &first_seed.request_id, "_assertion_proxy_a");
    assert_bound(&second, &second_seed.request_id, "_assertion_proxy_b");
    assert_ne!(first_response, second_response);
}

fn assert_bound(output: &ValidationOutput, request_id: &str, assertion_id: &str) {
    assert!(output.valid, "{}", output.error);
    assert_eq!(output.assertion_id, assertion_id);
    assert_eq!(output.response_in_response_to, request_id);
}

#[test]
fn distinguishes_mutated_assertions_with_the_independent_validator() {
    let metadata = prepare();
    let seed = corpus_seed("valid-authn-request-a.json");
    let cases = [
        (Mutation::Unsigned, "not signed"),
        (Mutation::WrongKey, "Signature validation failed"),
        (Mutation::Expired, "expired"),
        (Mutation::WrongAudience, "not a valid audience"),
        (Mutation::WrongDestination, "instead of"),
    ];
    for (mutation, expected) in cases {
        let assertion_id = format!("_assertion_{mutation:?}");
        let (output, _response) = sign_and_validate(
            &seed.saml_request,
            &seed.request_id,
            assertion(Some(mutation), &assertion_id),
            &metadata,
        );
        assert!(!output.valid, "{mutation:?} must be rejected");
        assert!(
            output.error.contains(expected),
            "{mutation:?}: {}",
            output.error
        );
    }
}

#[test]
fn binds_subject_confirmation_only_responses_through_the_assertion() {
    let metadata = prepare();
    let seed = corpus_seed("valid-authn-request-a.json");
    let (output, response) = sign_and_validate(
        &seed.saml_request,
        &seed.request_id,
        assertion(Some(Mutation::SubjectConfirmationOnly), "_assertion_scd"),
        &metadata,
    );
    assert!(output.valid, "{}", output.error);
    assert!(output.response_in_response_to.is_empty());
    let expected = format!("InResponseTo=\"{}\"", seed.request_id);
    assert!(response.contains(expected.as_str()));
}

#[test]
fn reuses_the_exact_signed_output_for_replay_steps() {
    let _metadata = prepare();
    let seed = corpus_seed("valid-authn-request-a.json");
    let assertion = assertion(None, "_assertion_replay");
    let mut signer = SamlSigner::new();
    let first = signer
        .form_body(&assertion, &seed.saml_request)
        .expect("the first body signs");
    let second = signer
        .form_body(&assertion, &seed.saml_request)
        .expect("the replay body builds");
    assert_eq!(first, second);
    let stub = StubServer::start(vec![
        redirect(&seed.saml_request),
        answer(200, Vec::new(), ""),
        answer(200, Vec::new(), ""),
    ]);
    let steps = transcript(&[assertion.clone(), assertion]);
    let responses = run_transcript(&stub, &steps);
    assert_eq!(responses.len(), 3);
    let first_body = stub.received_body(1).expect("the first post arrives");
    assert!(!first_body.is_empty());
    assert_eq!(Some(first_body), stub.received_body(2));
}

#[test]
fn rejects_dtd_entity_expansion_and_non_request_documents() {
    let dtd = deflated(
        "<!DOCTYPE samlp:AuthnRequest [<!ENTITY boom \"boom\">]>\
         <samlp:AuthnRequest xmlns:samlp=\"urn:oasis:names:tc:SAML:2.0:protocol\" \
         ID=\"_dtd\">&boom;</samlp:AuthnRequest>",
    );
    assert!(matches!(
        decode_authn_request_id(&dtd),
        Err(SamlIdpError::DocType)
    ));
    let metadata = deflated(
        "<EntityDescriptor xmlns=\"urn:oasis:names:tc:SAML:2.0:metadata\" entityID=\"x\"/>",
    );
    assert!(matches!(
        decode_authn_request_id(&metadata),
        Err(SamlIdpError::NotAuthnRequest)
    ));
    let idless =
        deflated("<samlp:AuthnRequest xmlns:samlp=\"urn:oasis:names:tc:SAML:2.0:protocol\"/>");
    assert!(matches!(
        decode_authn_request_id(&idless),
        Err(SamlIdpError::MissingRequestId)
    ));
    assert!(matches!(
        decode_authn_request_id(b"!!!"),
        Err(SamlIdpError::RequestBase64)
    ));
    let oversized = vec![b'A'; MAX_REQUEST_BYTES.saturating_add(1)];
    assert!(matches!(
        decode_authn_request_id(&oversized),
        Err(SamlIdpError::RequestTooLarge)
    ));
}

#[test]
fn rejects_saml_steps_that_cannot_be_bound_before_proxies_start() {
    let mut undeclared = assertion(None, "_assertion_undeclared");
    undeclared.request_id_capture = "ghost".to_owned();
    let steps = transcript(&[undeclared]);
    let error = validate_steps(Path::new("<case>"), &steps)
        .expect_err("an undeclared capture reference is rejected");
    assert!(error.to_string().contains("undeclared capture"));
    let mut steps = transcript(&[assertion(None, "_assertion_body")]);
    steps.get_mut(1).expect("the saml step exists").request.body =
        serde_json::json!({"probe": true});
    let error =
        validate_steps(Path::new("<case>"), &steps).expect_err("a request body is rejected");
    assert!(error.to_string().contains("cannot be combined"));
}

#[test]
fn replays_the_saml_oracle_input_corpus() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let decoded = replay_authn_request_corpus(&manifest.join("fuzz/corpus/saml_oracle_input"));
    assert!(decoded >= 2);
    assert_case_corpus_rejects(&manifest.join("fuzz/corpus/http_case"));
}

fn replay_authn_request_corpus(root: &Path) -> usize {
    let mut paths = fs::read_dir(root)
        .expect("the corpus folder reads")
        .map(|entry| entry.expect("a corpus entry").path())
        .collect::<Vec<_>>();
    paths.sort();
    assert!(!paths.is_empty());
    let mut decoded = 0_usize;
    for path in &paths {
        let bytes = fs::read(path).expect("a seed reads");
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if let Ok(seed) = serde_json::from_slice::<CorpusSeed>(&bytes) {
            assert_eq!(
                decode_authn_request_id(seed.saml_request.as_bytes()).expect("the seed decodes"),
                seed.request_id,
                "{}",
                path.display()
            );
            decoded = decoded.saturating_add(1);
        } else if name.ends_with(".yaml") {
            let steps = decode_case(path, &bytes)
                .expect("the case seed decodes")
                .steps
                .expect("the case seed has steps");
            assert!(
                steps.iter().any(|step| step.saml_assertion.is_some()),
                "{} carries no saml assertion",
                path.display()
            );
        } else if let Ok(id) = decode_authn_request_id(&bytes) {
            panic!("{} should be rejected, decoded {id}", path.display());
        }
    }
    decoded
}

fn assert_case_corpus_rejects(root: &Path) {
    for entry in fs::read_dir(root).expect("the http case corpus reads") {
        let path = entry.expect("a case entry").path();
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if !name.starts_with("saml-assertion-") {
            continue;
        }
        let bytes = fs::read(&path).expect("a case seed reads");
        if let Ok(case) = decode_case(&path, &bytes) {
            panic!("{} was accepted as \"{}\"", path.display(), case.name);
        }
    }
}

fn prepare() -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    sync_litellm(&root).expect("the litellm environment syncs");
    let signer = SamlSigner::new();
    ensure_keypair(&signer.directory).expect("the fixture keypair exists");
    fs::read_to_string(signer.directory.join("metadata.xml")).expect("the metadata reads")
}

fn corpus_seed(name: &str) -> CorpusSeed {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fuzz/corpus/saml_oracle_input")
        .join(name);
    serde_json::from_slice(&fs::read(path).expect("the seed reads")).expect("the seed parses")
}

fn assertion(mutation: Option<Mutation>, assertion_id: &str) -> SamlAssertion {
    SamlAssertion {
        subject: "alice@example.com".to_owned(),
        attributes: BTreeMap::from([
            ("email".to_owned(), vec!["alice@example.com".to_owned()]),
            ("role".to_owned(), vec!["internal_user".to_owned()]),
        ]),
        audience: SP_ENTITY.to_owned(),
        destination: format!("{PROXY_BASE}/sso/key/generate"),
        request_id_capture: "authn".to_owned(),
        assertion_id: assertion_id.to_owned(),
        not_before_offset_seconds: -60,
        not_on_or_after_offset_seconds: 300,
        relay_state: Some("/ui".to_owned()),
        mutation,
    }
}

fn transcript(assertions: &[SamlAssertion]) -> Vec<Step> {
    let mut steps = vec![redirect_step("authn")];
    steps.extend(
        assertions
            .iter()
            .map(|assertion| saml_step(assertion.clone(), "authn")),
    );
    steps
}

fn step(method: &str, capture: Option<&str>, assertion: Option<SamlAssertion>) -> Step {
    Step {
        request: Request {
            method: method.to_owned(),
            path: "/sso/key/generate".to_owned(),
            headers: BTreeMap::new(),
            body: Value::Null,
            body_kind: BodyKind::Json,
        },
        compare: Compare::default(),
        captures: capture
            .map(|name| Capture {
                name: name.to_owned(),
                source: CaptureSource::LocationQuery {
                    name: "SAMLRequest".to_owned(),
                },
                secret: false,
            })
            .into_iter()
            .collect(),
        form: false,
        saml_assertion: assertion,
    }
}

fn redirect_step(capture_name: &str) -> Step {
    step("GET", Some(capture_name), None)
}

fn saml_step(mut assertion: SamlAssertion, capture_name: &str) -> Step {
    assertion.request_id_capture = capture_name.to_owned();
    step("POST", None, Some(assertion))
}

fn redirect(encoded: &str) -> StubAnswer {
    let mut query = form_urlencoded::Serializer::new(String::new());
    query.append_pair("SAMLRequest", encoded);
    answer(
        303,
        vec![("location", format!("/idp/sso?{}", query.finish()))],
        "",
    )
}

fn run_transcript(stub: &StubServer, steps: &[Step]) -> Vec<Response> {
    run_case_steps(&client().expect("the client builds"), &stub.base_url, steps)
        .expect("the transcript runs")
}

fn sign_and_validate(
    encoded: &str,
    expected_id: &str,
    assertion: SamlAssertion,
    metadata: &str,
) -> (ValidationOutput, String) {
    let stub = StubServer::start(vec![redirect(encoded), answer(200, Vec::new(), "")]);
    let responses = run_transcript(&stub, &transcript(&[assertion]));
    assert_eq!(responses.len(), 2);
    assert_eq!(
        stub.received_header(1, "content-type").as_deref(),
        Some("application/x-www-form-urlencoded")
    );
    let body = stub.received_body(1).expect("the callback body arrives");
    validate_body(&body, expected_id, metadata)
}

fn validate_body(body: &[u8], expected_id: &str, metadata: &str) -> (ValidationOutput, String) {
    let response = form_value(body, "SAMLResponse");
    assert!(!response.is_empty());
    let destination = format!("{PROXY_BASE}/sso/key/generate");
    let output = validate(&ValidationInput {
        saml_response: &response,
        request_id: expected_id,
        audience: SP_ENTITY,
        destination: &destination,
        metadata,
    });
    (output, response)
}

fn validate(input: &ValidationInput<'_>) -> ValidationOutput {
    let document = serde_json::to_vec(input).expect("the validation input serializes");
    let output = run_fixture_script("validate.py", &document).expect("the validator runs");
    serde_json::from_slice(&output).expect("the validator output parses")
}

fn form_value(body: &[u8], name: &str) -> String {
    form_urlencoded::parse(body)
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
        .unwrap_or_default()
}

fn deflated(xml: &str) -> Vec<u8> {
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(xml.as_bytes())
        .expect("the encoder writes");
    STANDARD
        .encode(encoder.finish().expect("the encoder finishes"))
        .into_bytes()
}
