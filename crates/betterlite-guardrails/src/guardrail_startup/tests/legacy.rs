struct FakeStarter {
    started: Vec<String>,
    calls: Vec<LegacyPlan>,
    fail: bool,
}

impl FakeStarter {
    fn with_started(started: &[&str]) -> Self {
        Self {
            started: started.iter().map(|name| (*name).to_owned()).collect(),
            calls: Vec::new(),
            fail: false,
        }
    }
}

impl LegacyCallbackStarter for FakeStarter {
    fn is_started(&self, callback: &str) -> bool {
        self.started.iter().any(|name| name == callback)
    }

    fn start(
        &mut self,
        callbacks: &[String],
        callback_args: &CallbackArgs,
    ) -> Result<(), StartupError> {
        self.calls.push(LegacyPlan {
            callbacks: callbacks.to_vec(),
            callback_args: callback_args.clone(),
        });
        if self.fail {
            return Err(StartupError::Legacy {
                message: "boom".to_owned(),
            });
        }
        Ok(())
    }
}

fn legacy_settings(yaml: &str) -> LegacySettings {
    let raw: LegacyRaw = serde_saphyr::from_str(yaml).unwrap();
    raw.convert(&BTreeMap::<String, String>::new()).unwrap()
}

fn legacy_items(guardrails: &str) -> Vec<GuardrailItem> {
    legacy_settings(&format!("litellm_settings:\n  guardrails:\n{guardrails}")).items
}

const PII_WITH_ARGS: &str = "    - pii:\n        callbacks: [presidio, lakera_prompt_injection]\n        default_on: true\n        callback_args:\n          presidio:\n            language: en\n";
const NOT_DEFAULT_ON: &str = "    - pii:\n        callbacks: [presidio]\n        callback_args:\n          presidio:\n            language: en\n";
const LOGGING_ONLY_PRESIDIO: &str = "    - pii:\n        callbacks: [presidio]\n        default_on: true\n        logging_only: true\n        callback_args:\n          presidio:\n            language: en\n";
const LOGGING_ONLY_OTHER: &str = "    - pii:\n        callbacks: [lakera]\n        default_on: true\n        logging_only: true\n";
const LOGGING_ONLY_FALSE: &str = "    - pii:\n        callbacks: [presidio]\n        default_on: true\n        logging_only: false\n";
const LOGGING_ONLY_NOT_DEFAULT_ON: &str =
    "    - pii:\n        callbacks: [presidio]\n        logging_only: true\n";
const SAME_NAME_TWICE: &str = "    - dup:\n        callbacks: [first, shared]\n        default_on: true\n        callback_args:\n          shared:\n            from: one\n    - other:\n        callbacks: [second, first]\n        default_on: true\n    - dup:\n        callbacks: [third]\n        default_on: true\n        callback_args:\n          shared:\n            from: three\n";
const SHALLOW_UPDATE: &str = "    - one:\n        callbacks: [a]\n        callback_args:\n          cb:\n            keep: 1\n            drop: 2\n          other:\n            x: 1\n    - two:\n        callbacks: [b]\n        callback_args:\n          cb:\n            only: 3\n";

#[rstest]
#[case::default_on_with_args(PII_WITH_ARGS, &[], &["presidio", "lakera_prompt_injection"], r#"{"presidio":{"language":"en"}}"#)]
#[case::not_default_on_still_collects_args(NOT_DEFAULT_ON, &[], &[], r#"{"presidio":{"language":"en"}}"#)]
#[case::started_callback_is_not_listed(PII_WITH_ARGS, &["presidio"], &["lakera_prompt_injection"], r#"{"presidio":{"language":"en"}}"#)]
#[case::logging_only_replaces_the_presidio_args(LOGGING_ONLY_PRESIDIO, &[], &["presidio"], r#"{"presidio":{"logging_only":true}}"#)]
#[case::logging_only_applies_when_presidio_is_started(LOGGING_ONLY_PRESIDIO, &["presidio"], &[], r#"{"presidio":{"logging_only":true}}"#)]
#[case::logging_only_ignores_other_callbacks(LOGGING_ONLY_OTHER, &[], &["lakera"], "{}")]
#[case::logging_only_false_keeps_the_args(LOGGING_ONLY_FALSE, &[], &["presidio"], "{}")]
#[case::logging_only_needs_default_on(LOGGING_ONLY_NOT_DEFAULT_ON, &[], &[], "{}")]
#[case::same_name_twice_feeds_the_plan_twice(SAME_NAME_TWICE, &["second"], &["first", "shared", "third"], r#"{"shared":{"from":"three"}}"#)]
#[case::args_replace_whole_top_level_keys(SHALLOW_UPDATE, &[], &[], r#"{"cb":{"only":3},"other":{"x":1}}"#)]
fn legacy_plan_matches_litellm(
    #[case] guardrails: &str,
    #[case] started: &[&str],
    #[case] expected_callbacks: &[&str],
    #[case] expected_args: &str,
) {
    let plan = plan_legacy_callbacks(
        &legacy_items(guardrails),
        &FakeStarter::with_started(started),
    );
    assert_eq!(plan.callbacks, expected_callbacks);
    assert_eq!(
        serde_json::to_string(&plan.callback_args).unwrap(),
        expected_args
    );
}

#[test]
fn legacy_plan_of_no_items_is_empty() {
    let plan = plan_legacy_callbacks(&[], &FakeStarter::with_started(&[]));
    assert!(plan.callbacks.is_empty());
    assert!(plan.callback_args.is_empty());
}

#[test]
fn legacy_start_calls_the_starter_once_with_the_plan() {
    let settings = legacy_settings(&format!(
        "litellm_settings:\n  guardrails:\n{PII_WITH_ARGS}"
    ));
    let mut starter = FakeStarter::with_started(&["lakera_prompt_injection"]);
    start_legacy_guardrails(&settings, &mut starter).unwrap();
    assert_eq!(starter.calls.len(), 1);
    assert_eq!(starter.calls[0].callbacks, ["presidio"]);
    let args = serde_json::to_string(&starter.calls[0].callback_args).unwrap();
    assert_eq!(args, r#"{"presidio":{"language":"en"}}"#);
}

#[rstest]
#[case::no_default_on(NOT_DEFAULT_ON, &[])]
#[case::everything_started(PII_WITH_ARGS, &["presidio", "lakera_prompt_injection"])]
fn legacy_start_does_not_call_the_starter_when_nothing_is_to_start(
    #[case] guardrails: &str,
    #[case] started: &[&str],
) {
    let settings = legacy_settings(&format!("litellm_settings:\n  guardrails:\n{guardrails}"));
    let mut starter = FakeStarter::with_started(started);
    start_legacy_guardrails(&settings, &mut starter).unwrap();
    assert!(starter.calls.is_empty());
}

#[test]
fn legacy_start_logs_and_returns_a_starter_failure() {
    let (captured, _guard) = capture();
    let settings = legacy_settings(&format!(
        "litellm_settings:\n  guardrails:\n{PII_WITH_ARGS}"
    ));
    let mut starter = FakeStarter::with_started(&[]);
    starter.fail = true;
    let failed = start_legacy_guardrails(&settings, &mut starter).unwrap_err();
    assert!(matches!(&failed, StartupError::Legacy { message } if message == "boom"));
    let text = logged(&captured);
    assert!(text.contains("ERROR"), "{text}");
    assert!(
        text.contains("error initializing guardrails boom"),
        "{text}"
    );
}

#[test]
fn legacy_start_without_guardrails_does_nothing() {
    let mut starter = FakeStarter::with_started(&[]);
    start_legacy_guardrails(&legacy_settings("litellm_settings: {}"), &mut starter).unwrap();
    assert!(starter.calls.is_empty());
}
