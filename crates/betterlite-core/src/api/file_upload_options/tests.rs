#![cfg(test)]

use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use tokio::runtime::Builder;

use super::error::{FileUploadOptionsError, ResourceKind};
use super::managed::{validate_managed_id, validate_managed_upload};
use super::{
    FileCreationParams, FileCreationRequest, RequestBodyRead, extract_file_creation_params,
};
use crate::api::file_routing_ids::ManagedResourceAccessChecker;
use crate::api::request_body::{JsonObject, ParsedJson, parse_json_text};
use crate::api::request_form::values::{FormPart, FormParts, PartValue, UploadFile};

const ENCODED_ID: &str = "bGl0ZWxsbV9wcm94eTpmaWxlO3VuaWZpZWRfaWQsdQ";
const NAMES_REQUIRED: &str = "target_model_names is required when require_managed_files is enabled in litellm_settings. Provide one or more model aliases via the target_model_names form field (e.g. target_model_names=my-model-alias).";
const MODEL_NOT_ALLOWED: &str = "model is not allowed when require_managed_files is enabled in litellm_settings. Uploads must go through managed files using target_model_names instead of the model parameter.";
const RAW_FILE: &str = "Raw provider file ids cannot be used when require_managed_files is enabled in litellm_settings. Use the LiteLLM managed file id returned when the file was created.";
const RAW_BATCH: &str = "Raw provider batch ids cannot be used when require_managed_files is enabled in litellm_settings. Use the LiteLLM managed batch id returned when the batch was created.";
const RAW_JOB: &str = "Raw provider fine-tuning job ids cannot be used when require_managed_files is enabled in litellm_settings. Use the LiteLLM managed fine-tuning job id returned when the fine-tuning job was created.";
const UNAVAILABLE: &str = "Managed resource ownership validation is unavailable.";

#[derive(Clone, Copy)]
enum Gate {
    Enabled,
    Disabled,
}

impl Gate {
    const fn enabled(self) -> bool {
        matches!(self, Self::Enabled)
    }
}

struct Fixture {
    body: Option<JsonObject>,
    query_params: BTreeMap<String, String>,
    headers: BTreeMap<String, String>,
    form_parts: FormParts,
    reads: Cell<usize>,
}

impl Fixture {
    fn new(
        body: Option<JsonObject>,
        query: &[(&str, &str)],
        headers: &[(&str, &str)],
        parts: &[FormPart],
    ) -> Self {
        Self {
            body,
            query_params: pair_map(query),
            headers: pair_map(headers),
            form_parts: FormParts {
                items: parts.to_vec(),
            },
            reads: Cell::new(0),
        }
    }

    fn extract(
        &self,
        request_body: Option<&JsonObject>,
        names_form: Option<&Json>,
        storage_form: Option<&str>,
    ) -> Result<FileCreationParams, FileUploadOptionsError> {
        let request = FileCreationRequest {
            query_params: &self.query_params,
            headers: &self.headers,
            form_parts: &self.form_parts,
            read_body: self,
        };
        extract_file_creation_params(&request, request_body, names_form, storage_form)
    }
}

impl RequestBodyRead for Fixture {
    fn request_body(&self) -> Option<JsonObject> {
        self.reads.set(self.reads.get().saturating_add(1));
        self.body.clone()
    }
}

fn pair_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn json_body(source: &str) -> JsonObject {
    match parse_json_text(source) {
        ParsedJson::Object(object) => object,
        ParsedJson::NotObject | ParsedJson::Invalid => panic!("the test source is a JSON object"),
    }
}

fn json_value(source: &str) -> Json {
    serde_json::from_str::<Json>(source).unwrap()
}

fn names(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

fn text_part(name: &str, text: &str) -> FormPart {
    FormPart {
        name: name.to_owned(),
        value: PartValue::Text(text.to_owned()),
    }
}

fn upload_part(name: &str, filename: &str) -> FormPart {
    FormPart {
        name: name.to_owned(),
        value: PartValue::Upload(UploadFile {
            filename: filename.to_owned(),
            content_type: None,
            content: Vec::new(),
            size: None,
        }),
    }
}

fn principal() -> Principal {
    Principal {
        principal_type: PrincipalType::Human,
        subject: "subject".to_owned(),
        issuer: None,
        audience: Vec::new(),
        user: None,
        organization: None,
        teams: Vec::new(),
        project: None,
        end_user: None,
        roles: Vec::new(),
        scopes: Vec::new(),
        auth_method: AuthMethod::ApiKey,
        credential_ref: CredentialRef::default(),
        network: NetworkContext::default(),
        source_key: SourceKey::default(),
    }
}

struct RecordingChecker {
    allowed: bool,
    file_id: Mutex<Option<String>>,
    object_id: Mutex<Option<String>>,
}

impl RecordingChecker {
    fn record(&self, target: &Mutex<Option<String>>, id: &str) -> bool {
        *target.lock().unwrap() = Some(id.to_owned());
        self.allowed
    }
}

#[async_trait]
impl ManagedResourceAccessChecker for RecordingChecker {
    async fn can_call_unified_file_id(
        &self,
        unified_file_id: &str,
        _principal: &Principal,
    ) -> bool {
        self.record(&self.file_id, unified_file_id)
    }

    async fn can_call_unified_object_id(
        &self,
        unified_object_id: &str,
        _principal: &Principal,
    ) -> bool {
        self.record(&self.object_id, unified_object_id)
    }
}

fn recording(allowed: bool) -> RecordingChecker {
    RecordingChecker {
        allowed,
        file_id: Mutex::new(None),
        object_id: Mutex::new(None),
    }
}

#[test]
fn creation_defaults_direct() {
    let params = FileCreationParams::default();
    assert_eq!(params.target_storage, "default");
    assert_eq!(params.target_model_names, Vec::<String>::new());
    assert_eq!(params.model, None);

    let empty = FileCreationParams::new(Some(String::new()), None, None);
    assert_eq!(empty.target_storage, "default");
    assert_eq!(empty.target_model_names, Vec::<String>::new());

    let padded = FileCreationParams::new(Some(" azure_storage ".to_owned()), None, None);
    assert_eq!(padded.target_storage, " azure_storage ");

    let spaces = FileCreationParams::new(Some("   ".to_owned()), None, None);
    assert_eq!(spaces.target_storage, "   ");

    let raw = names(&[" a ", "", "  b  "]);
    let normalized = FileCreationParams::new(None, Some(raw), None);
    assert_eq!(normalized.target_model_names, names(&["a", "b"]));
}

#[test]
fn creation_extraction() {
    let body = json_body("{\"model\": \" b \"}");
    let probe = Fixture::new(
        Some(body.clone()),
        &[("model", "q")],
        &[("x-litellm-model", "h")],
        &[],
    );
    let supplied = probe
        .extract(Some(&body), None, Some(" azure_storage "))
        .unwrap();
    assert_eq!(supplied.model.as_deref(), Some(" b "));
    assert_eq!(supplied.target_storage, "azure_storage");
    assert_eq!(probe.reads.get(), 0);

    let spaces = probe.extract(Some(&body), None, Some("   ")).unwrap();
    assert_eq!(spaces.target_storage, "default");
}

#[test]
fn creation_extraction_reads_a_missing_body_once() {
    let probe = Fixture::new(
        Some(json_body("{\"model\": \" b \"}")),
        &[("model", "q")],
        &[("x-litellm-model", "h")],
        &[],
    );
    let read = probe.extract(None, None, None).unwrap();
    assert_eq!(read.model.as_deref(), Some(" b "));
    assert_eq!(probe.reads.get(), 1);

    // a falsey read result is the empty body, so the request query decides the model
    let falsey = Fixture::new(None, &[("model", "q")], &[("x-litellm-model", "h")], &[]);
    let fallback = falsey.extract(None, None, None).unwrap();
    assert_eq!(fallback.model.as_deref(), Some("q"));
    assert_eq!(falsey.reads.get(), 1);
}

#[test]
fn model_falls_from_the_body_to_the_query_then_the_header() {
    let empty_body = json_body("{\"model\": \"\"}");
    let query = Fixture::new(
        Some(empty_body.clone()),
        &[("model", "q")],
        &[("x-litellm-model", "h")],
        &[],
    );
    assert_eq!(
        query
            .extract(Some(&empty_body), None, None)
            .unwrap()
            .model
            .as_deref(),
        Some("q")
    );

    let header = Fixture::new(
        Some(empty_body.clone()),
        &[("model", "")],
        &[("x-litellm-model", "h")],
        &[],
    );
    assert_eq!(
        header
            .extract(Some(&empty_body), None, None)
            .unwrap()
            .model
            .as_deref(),
        Some("h")
    );

    let blank = Fixture::new(None, &[("model", "")], &[("x-litellm-model", "")], &[]);
    assert_eq!(
        blank.extract(None, None, None).unwrap().model.as_deref(),
        Some("")
    );
}

#[test]
fn null_body_model_falls_through() {
    let null_body = json_body("{\"model\": null}");
    let null_query = Fixture::new(Some(null_body.clone()), &[("model", "q")], &[], &[]);
    assert_eq!(
        null_query
            .extract(Some(&null_body), None, None)
            .unwrap()
            .model
            .as_deref(),
        Some("q")
    );
    let null_only = Fixture::new(Some(null_body.clone()), &[], &[], &[]);
    assert_eq!(
        null_only
            .extract(Some(&null_body), None, None)
            .unwrap()
            .model,
        None
    );
}

#[test]
fn control_characters_count_as_strippable_space() {
    let raw = names(&["\u{1c}a\u{1f}", "\u{b}b"]);
    let direct = FileCreationParams::new(None, Some(raw), None);
    assert_eq!(direct.target_model_names, names(&["a", "b"]));

    let probe = Fixture::new(None, &[], &[], &[]);
    let form = Json::Text("\u{1c} c \u{1f}".to_owned());
    let extracted = probe
        .extract(None, Some(&form), Some("\u{1c} azure_storage \u{1f}"))
        .unwrap();
    assert_eq!(extracted.target_model_names, names(&["c"]));
    assert_eq!(extracted.target_storage, "azure_storage");
}

#[rstest]
#[case::comma("\" a, b,,a \"", &["a", "b", "a"])]
#[case::list("[\" a \", 0, 2, \" \"]", &["a", "2"])]
#[case::literal("\"[\\\"a\\\",\\\"b\\\"]\"", &["[\"a\"", "\"b\"]"])]
#[case::empty("\"\"", &[])]
#[case::null("null", &[])]
#[case::boolean("true", &[])]
#[case::number("5", &[])]
#[case::object("{\"a\": true}", &[])]
fn simple_targets(#[case] source: &str, #[case] expected: &[&str]) {
    let form = json_value(source);
    let probe = Fixture::new(None, &[], &[], &[]);
    let params = probe.extract(None, Some(&form), None).unwrap();
    assert_eq!(params.target_model_names, names(expected));
}

#[test]
fn form_fallback_needs_an_empty_simple_result() {
    let parts = vec![text_part("target_model_names", "from_form")];
    let probe = Fixture::new(None, &[], &[], &parts);
    let explicit = json_value("\" b \"");
    assert_eq!(
        probe
            .extract(None, Some(&explicit), None)
            .unwrap()
            .target_model_names,
        names(&["b"])
    );
    let blank = json_value("\" \"");
    assert_eq!(
        probe
            .extract(None, Some(&blank), None)
            .unwrap()
            .target_model_names,
        names(&["from_form"])
    );
}

#[test]
fn an_empty_list_text_still_asks_for_the_form() {
    let parts = vec![text_part("target_model_names", "from_form")];
    let probe = Fixture::new(None, &[], &[], &parts);
    let empty_text = Json::List(vec![Json::Text(String::new())]);
    assert_eq!(
        probe
            .extract(None, Some(&empty_text), None)
            .unwrap()
            .target_model_names,
        names(&["from_form"])
    );
}

#[test]
fn repeated_targets() {
    let parts = [
        text_part("target_model_names[]", "a"),
        text_part("target_model_names[0]", " b,a "),
        text_part("target_model_names", "a"),
        text_part("target_model_names[xyz]", "b"),
        text_part("target_model_names_extra", "c"),
        text_part("target_model_names[unterminated", "d"),
        text_part("target_model_names[]", "a"),
        upload_part("target_model_names[]", "f.txt"),
        text_part("other[]", "z"),
    ];
    let form = FormParts {
        items: parts.to_vec(),
    };
    assert_eq!(
        super::collected_form_names(&form),
        names(&["a", "b", "a", "a", "b", "a"])
    );
    let probe = Fixture::new(None, &[], &[], &parts);
    let params = probe.extract(None, None, None).unwrap();
    assert_eq!(params.target_model_names, names(&["a", "b"]));
}

#[rstest]
#[case::missing_names(&[], Some("m"), Gate::Enabled, Some(NAMES_REQUIRED))]
#[case::model_with_names(&["a"], Some("m"), Gate::Enabled, Some(MODEL_NOT_ALLOWED))]
#[case::empty_model(&["a"], Some(""), Gate::Enabled, None)]
#[case::missing_model(&["a"], None, Gate::Enabled, None)]
#[case::disabled_missing(&[], Some("m"), Gate::Disabled, None)]
#[case::disabled_model(&["a"], Some("m"), Gate::Disabled, None)]
fn managed_upload_errors(
    #[case] target_model_names: &[&str],
    #[case] model: Option<&str>,
    #[case] gate: Gate,
    #[case] expected: Option<&str>,
) {
    let outcome = validate_managed_upload(gate.enabled(), &names(target_model_names), model);
    assert_eq!(
        outcome
            .err()
            .map(|error| (error.status(), error.to_string())),
        expected.map(|detail| (400, detail.to_owned()))
    );
}

#[rstest]
#[case::missing(Some(""), ResourceKind::File, Gate::Enabled, None)]
#[case::absent(None, ResourceKind::Batch, Gate::Enabled, None)]
#[case::raw_file(Some("file-1"), ResourceKind::File, Gate::Enabled, Some((400, RAW_FILE)))]
#[case::raw_batch(Some("batch-1"), ResourceKind::Batch, Gate::Enabled, Some((400, RAW_BATCH)))]
#[case::raw_job(Some("job-1"), ResourceKind::FineTuningJob, Gate::Enabled, Some((400, RAW_JOB)))]
#[case::encoded(Some(ENCODED_ID), ResourceKind::File, Gate::Enabled, Some((500, UNAVAILABLE)))]
#[case::disabled_encoded(Some(ENCODED_ID), ResourceKind::File, Gate::Disabled, None)]
#[case::disabled_raw(Some("file-1"), ResourceKind::Batch, Gate::Disabled, None)]
fn managed_id_gate(
    #[case] resource_id: Option<&str>,
    #[case] resource_kind: ResourceKind,
    #[case] gate: Gate,
    #[case] expected: Option<(u16, &str)>,
) {
    let runtime = Builder::new_current_thread().build().unwrap();
    let outcome = runtime.block_on(validate_managed_id(
        gate.enabled(),
        resource_id,
        resource_kind,
        None,
        &principal(),
    ));
    assert_eq!(
        outcome
            .err()
            .map(|error| (error.status(), error.to_string())),
        expected.map(|(status, detail)| (status, detail.to_owned()))
    );
}

#[test]
fn managed_id_checker_contract() {
    let runtime = Builder::new_current_thread().build().unwrap();
    let user = principal();
    let permit_files = recording(true);
    let file_outcome = runtime.block_on(validate_managed_id(
        true,
        Some(ENCODED_ID),
        ResourceKind::File,
        Some(&permit_files),
        &user,
    ));
    assert_eq!(file_outcome, Ok(()));
    assert_eq!(
        permit_files.file_id.lock().unwrap().take(),
        Some(ENCODED_ID.to_owned())
    );
    assert_eq!(permit_files.object_id.lock().unwrap().take(), None);
}

#[test]
fn managed_id_checker_denies_a_job_through_the_object_method() {
    let runtime = Builder::new_current_thread().build().unwrap();
    let deny_job = recording(false);
    let job_outcome = runtime.block_on(validate_managed_id(
        true,
        Some(ENCODED_ID),
        ResourceKind::FineTuningJob,
        Some(&deny_job),
        &principal(),
    ));
    assert_eq!(
        job_outcome
            .err()
            .map(|error| (error.status(), error.to_string())),
        Some((
            403,
            "The caller does not have access to this managed fine-tuning job id.".to_owned()
        ))
    );
    assert_eq!(
        deny_job.object_id.lock().unwrap().take(),
        Some(ENCODED_ID.to_owned())
    );
    assert_eq!(deny_job.file_id.lock().unwrap().take(), None);
}

#[test]
fn managed_id_checker_denies_a_batch_through_the_object_method() {
    let runtime = Builder::new_current_thread().build().unwrap();
    let deny_batch = recording(false);
    let batch_outcome = runtime.block_on(validate_managed_id(
        true,
        Some(ENCODED_ID),
        ResourceKind::Batch,
        Some(&deny_batch),
        &principal(),
    ));
    assert_eq!(
        batch_outcome.err().map(|error| error.to_string()),
        Some("The caller does not have access to this managed batch id.".to_owned())
    );
    assert_eq!(
        deny_batch.object_id.lock().unwrap().take(),
        Some(ENCODED_ID.to_owned())
    );
    assert_eq!(deny_batch.file_id.lock().unwrap().take(), None);
}

#[test]
fn number_names_keep_their_json_text() {
    let form = json_value("[2, 2.5]");
    let probe = Fixture::new(None, &[], &[], &[]);
    let params = probe.extract(None, Some(&form), None).unwrap();
    assert_eq!(params.target_model_names, names(&["2", "2.5"]));
}

#[test]
fn list_bools_and_objects_are_rejected() {
    let boolean = Json::List(vec![Json::Text("a".to_owned()), Json::Bool(true)]);
    let probe = Fixture::new(None, &[], &[], &[]);
    assert_eq!(
        probe.extract(None, Some(&boolean), None),
        Err(FileUploadOptionsError::NamesNotText)
    );
    let mut entries = IndexMap::new();
    entries.insert("a".to_owned(), Json::Bool(true));
    let object = Json::List(vec![Json::Object(entries)]);
    assert_eq!(
        probe.extract(None, Some(&object), None),
        Err(FileUploadOptionsError::NamesNotText)
    );
    let nested = Json::List(vec![Json::List(vec![Json::Text("a".to_owned())])]);
    assert_eq!(
        probe.extract(None, Some(&nested), None),
        Err(FileUploadOptionsError::NamesNotText)
    );
    let empty = Json::List(vec![Json::List(Vec::new()), Json::Object(IndexMap::new())]);
    assert_eq!(
        probe
            .extract(None, Some(&empty), None)
            .unwrap()
            .target_model_names,
        Vec::<String>::new()
    );
}

#[test]
fn body_model_must_be_text() {
    let body = json_body("{\"model\": 5}");
    let probe = Fixture::new(None, &[], &[], &[]);
    assert_eq!(
        probe.extract(Some(&body), None, None),
        Err(FileUploadOptionsError::ModelNotText)
    );
    assert_eq!(FileUploadOptionsError::ModelNotText.status(), 400);
}
