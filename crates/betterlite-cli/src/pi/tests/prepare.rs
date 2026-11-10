const MODELS: &str = r#"{"data":[{"id":"gpt-4o"},{"id":"claude"}]}"#;
const LIMITS: &str =
    r#"{"data":[{"model_group":"gpt-4o","max_input_tokens":128000,"max_output_tokens":4096}]}"#;

fn prepare(
    environment: &FakeEnvironment,
    get: &ScriptedGet,
    files: &dyn FileAccess,
    notice: &mut Vec<u8>,
) -> Result<Vec<String>, PiSyncError> {
    let mut context = PreparePiContext {
        environment,
        get,
        files,
        notice,
    };
    prepare_pi(BASE_URL, &api_key(), &mut context)
}

#[test]
fn prepare_pi_writes_models_and_returns_the_first_model_argument() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let get = ScriptedGet::new(vec![body(200, MODELS), body(200, LIMITS)]);
    let files = ScriptedFiles::new(FileFault::None);
    let mut notice = Vec::new();
    let arguments = prepare(&environment, &get, &files, &mut notice).unwrap();
    assert_eq!(
        arguments,
        vec!["--model".to_owned(), "litellm/gpt-4o".to_owned()]
    );
    let path = home.path().join(".pi").join("agent").join("models.json");
    let document = read_json(&path);
    assert_eq!(
        document
            .pointer("/providers/litellm/baseUrl")
            .unwrap()
            .as_str(),
        Some("https://proxy.test/v1")
    );
    assert_eq!(
        document
            .pointer("/providers/litellm/models/0/contextWindow")
            .unwrap()
            .as_i64(),
        Some(128_000)
    );
    assert_eq!(
        String::from_utf8(notice).unwrap(),
        format!("litellm: synced 2 proxy models into {}\n", path.display())
    );
    assert_eq!(
        get.urls(),
        vec![
            "https://proxy.test/v1/models".to_owned(),
            "https://proxy.test/model_group/info".to_owned(),
        ]
    );
}

#[test]
fn prepare_pi_writes_under_pi_coding_agent_dir() {
    let directory = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_override(directory.path());
    let get = ScriptedGet::new(vec![body(200, MODELS), body(200, LIMITS)]);
    let files = ScriptedFiles::new(FileFault::None);
    let mut notice = Vec::new();
    prepare(&environment, &get, &files, &mut notice).unwrap();
    assert!(directory.path().join("models.json").exists());
}

#[test]
fn prepare_pi_launches_when_limit_discovery_fails() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let get = ScriptedGet::new(vec![body(200, MODELS), body(500, "boom")]);
    let files = ScriptedFiles::new(FileFault::None);
    let mut notice = Vec::new();
    let arguments = prepare(&environment, &get, &files, &mut notice).unwrap();
    assert_eq!(
        arguments,
        vec!["--model".to_owned(), "litellm/gpt-4o".to_owned()]
    );
    let path = home.path().join(".pi").join("agent").join("models.json");
    let document = read_json(&path);
    assert!(
        document
            .pointer("/providers/litellm/models/0/contextWindow")
            .is_none()
    );
}

#[test]
fn prepare_pi_reports_an_empty_listing_as_nothing_to_run() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let get = ScriptedGet::new(vec![body(200, r#"{"data":[]}"#)]);
    let files = ScriptedFiles::new(FileFault::None);
    let mut notice = Vec::new();
    let error = prepare(&environment, &get, &files, &mut notice).unwrap_err();
    assert_eq!(error.kind(), Some(ListingFailure::Empty));
    assert!(error.to_string().contains("pi would have nothing to run"));
    assert_eq!(get.urls(), vec!["https://proxy.test/v1/models".to_owned()]);
    assert!(!home.path().join(".pi/agent/models.json").exists());
}

#[test]
fn prepare_pi_reports_a_listing_failure_without_writing() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let get = ScriptedGet::new(vec![body(401, "{}")]);
    let files = ScriptedFiles::new(FileFault::None);
    let mut notice = Vec::new();
    let error = prepare(&environment, &get, &files, &mut notice).unwrap_err();
    assert_eq!(error.kind(), Some(ListingFailure::Rejected));
    assert_eq!(error.status(), Some(401));
    assert!(!home.path().join(".pi/agent/models.json").exists());
}

#[test]
fn prepare_pi_reports_a_failed_models_write() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let get = ScriptedGet::new(vec![body(200, MODELS), body(200, LIMITS)]);
    let files = ScriptedFiles::new(FileFault::Write);
    let mut notice = Vec::new();
    let error = prepare(&environment, &get, &files, &mut notice).unwrap_err();
    assert_eq!(error.kind(), None);
    assert!(matches!(error, PiSyncError::WriteModels { .. }));
    assert!(notice.is_empty());
}

#[test]
fn prepare_pi_requires_a_models_file_location() {
    let environment = FakeEnvironment::without_home();
    let get = ScriptedGet::new(vec![body(200, MODELS), body(200, LIMITS)]);
    let files = ScriptedFiles::new(FileFault::None);
    let mut notice = Vec::new();
    let error = prepare(&environment, &get, &files, &mut notice).unwrap_err();
    assert_eq!(error.kind(), None);
    assert!(matches!(error, PiSyncError::NoHome));
    assert_eq!(get.urls().len(), 2);
}
