#![cfg(test)]

use rstest::rstest;
use serde_json::{Map, Value, json};

use super::error::RouterSettingsError;
use super::target::RouterSettingsTarget;
use super::{RUNTIME_UPDATABLE_ROUTER_SETTINGS, RouterRuntimeSettings, SettingsView};

type Calls = Vec<(&'static str, Value)>;

#[derive(Default)]
struct Recorder {
    calls: Calls,
    fail_groups: bool,
    fail_strategy: bool,
}

impl RouterSettingsTarget for Recorder {
    fn set_optional_pre_call_checks(&mut self, checks: &Value) -> Result<(), RouterSettingsError> {
        self.calls
            .push(("set_optional_pre_call_checks", checks.clone()));
        Ok(())
    }

    fn init_routing_strategy(
        &mut self,
        strategy: &Value,
        args: &Value,
    ) -> Result<(), RouterSettingsError> {
        self.calls.push((
            "init_routing_strategy",
            json!({"strategy": strategy, "args": args}),
        ));
        if self.fail_strategy {
            return Err(RouterSettingsError::Target {
                name: "init_routing_strategy".to_owned(),
                message: "unknown strategy".to_owned(),
            });
        }
        Ok(())
    }

    fn apply_updated_strategy_args(&mut self) -> Result<(), RouterSettingsError> {
        self.calls
            .push(("apply_updated_strategy_args", Value::Null));
        Ok(())
    }

    fn apply_lar1(&mut self, args: Option<&Value>) -> Result<(), RouterSettingsError> {
        self.calls.push((
            "apply_lar1",
            json!({"absent": args.is_none(), "args": args}),
        ));
        Ok(())
    }

    fn init_routing_groups(&mut self, input: Option<&Value>) -> Result<(), RouterSettingsError> {
        self.calls
            .push(("init_routing_groups", input.cloned().unwrap_or(Value::Null)));
        if self.fail_groups {
            return Err(RouterSettingsError::Target {
                name: "init_routing_groups".to_owned(),
                message: "invalid routing_groups".to_owned(),
            });
        }
        Ok(())
    }
}

fn router(values: Value) -> RouterRuntimeSettings {
    RouterRuntimeSettings {
        values: values.as_object().cloned().unwrap(),
        routing_groups_input: None,
    }
}

fn pairs(entries: Value) -> Vec<(String, Value)> {
    entries
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn update(router: &mut RouterRuntimeSettings, entries: Value) -> Recorder {
    let mut recorder = Recorder::default();
    router
        .update_settings(&mut recorder, &pairs(entries))
        .unwrap();
    recorder
}

fn shown(router: &RouterRuntimeSettings) -> Map<String, Value> {
    router.get_settings(&SettingsView::default())
}

fn full_values() -> Value {
    json!({
        "routing_strategy_args": {},
        "routing_strategy": "simple-shuffle",
        "allowed_fails": 3,
        "cooldown_time": 5,
        "num_retries": 2,
        "timeout": 600,
        "retry_after": 0,
        "fallbacks": [{"gpt": ["claude"]}],
        "context_window_fallbacks": [],
        "model_group_retry_policy": null,
        "retry_policy": null,
        "model_group_alias": {},
        "enable_weighted_failover": false,
        "enable_tag_filtering": false,
        "tag_routing_prefix": "",
    })
}

#[test]
fn get_settings_lists_the_stored_names_and_the_routing_groups_but_no_max_retries() {
    let settings = router(full_values()).get_settings(&SettingsView::default());

    let mut names: Vec<&str> = settings.keys().map(String::as_str).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "allowed_fails",
            "context_window_fallbacks",
            "cooldown_time",
            "enable_tag_filtering",
            "enable_weighted_failover",
            "fallbacks",
            "model_group_alias",
            "model_group_retry_policy",
            "num_retries",
            "retry_after",
            "retry_policy",
            "routing_groups",
            "routing_strategy",
            "routing_strategy_args",
            "tag_routing_prefix",
            "timeout",
        ]
    );
    assert_eq!(settings["timeout"], json!(600));
    assert_eq!(settings["routing_groups"], json!([]));
}

#[test]
fn get_settings_skips_a_name_the_router_never_stored() {
    let settings = router(json!({"timeout": 5})).get_settings(&SettingsView::default());

    let mut names: Vec<&str> = settings.keys().map(String::as_str).collect();
    names.sort_unstable();
    assert_eq!(names, ["routing_groups", "timeout"]);
}

#[test]
fn get_settings_hides_values_outside_the_list() {
    let mut values = full_values();
    values["optional_pre_call_checks"] = json!(["prompt_caching"]);
    values["routing_groups"] = json!([{"name": "stored"}]);

    let settings = router(values).get_settings(&SettingsView::default());

    assert!(!settings.contains_key("optional_pre_call_checks"));
    assert_eq!(settings["routing_groups"], json!([]));
}

#[test]
fn get_settings_adds_the_dumped_routing_groups() {
    let view = SettingsView {
        routing_groups: vec![json!({"name": "fast"}), json!({"name": "cheap"})],
        latency_routing_args: None,
    };

    let settings = router(full_values()).get_settings(&view);

    assert_eq!(
        settings["routing_groups"],
        json!([{"name": "fast"}, {"name": "cheap"}])
    );
}

#[rstest]
#[case::latency_with_args("latency-based-routing", Some("{\"ttl\":3600}"), json!("{\"ttl\":3600}"))]
#[case::latency_without_args("latency-based-routing", None, json!({"ttl": 1}))]
#[case::other_strategy_keeps_the_object("usage-based-routing", Some("{\"ttl\":3600}"), json!({"ttl": 1}))]
fn get_settings_shows_latency_args_as_text_only_for_the_latency_strategy(
    #[case] strategy: &str,
    #[case] latency_args: Option<&str>,
    #[case] expected: Value,
) {
    let mut values = full_values();
    values["routing_strategy"] = json!(strategy);
    values["routing_strategy_args"] = json!({"ttl": 1});
    let view = SettingsView {
        routing_groups: vec![],
        latency_routing_args: latency_args.map(str::to_owned),
    };

    let settings = router(values).get_settings(&view);

    assert_eq!(settings["routing_strategy_args"], expected);
    assert_eq!(settings["timeout"], json!(600));
}

#[test]
fn get_settings_shows_max_retries_once_an_update_sets_it() {
    let mut settings = router(full_values());
    assert!(!shown(&settings).contains_key("max_retries"));

    update(&mut settings, json!({"max_retries": 4}));

    assert_eq!(shown(&settings)["max_retries"], json!(4));
}

#[test]
fn get_settings_never_shows_a_name_update_skipped() {
    let mut settings = router(full_values());

    let recorder = update(&mut settings, json!({"cache": true, "num_retries": 9}));

    assert!(recorder.calls.is_empty());
    assert!(!shown(&settings).contains_key("cache"));
    assert_eq!(shown(&settings)["num_retries"], json!(9));
}

#[test]
fn get_settings_names_every_updatable_setting_once() {
    let mut names = RUNTIME_UPDATABLE_ROUTER_SETTINGS.to_vec();
    names.sort_unstable();
    names.dedup();

    assert_eq!(names.len(), 18);
}

#[test]
fn get_settings_stores_the_other_names_as_given() {
    let mut settings = router(full_values());

    update(
        &mut settings,
        json!({
            "fallbacks": [{"a": ["b"]}],
            "context_window_fallbacks": [{"c": ["d"]}],
            "model_group_retry_policy": {"gpt": {"DefaultRetries": 2}},
            "model_group_alias": {"x": ["y"]},
            "enable_weighted_failover": true,
            "enable_tag_filtering": true,
            "tag_routing_prefix": "tag:",
            "max_retries": 3.5,
        }),
    );

    let view = shown(&settings);
    assert_eq!(view["fallbacks"], json!([{"a": ["b"]}]));
    assert_eq!(view["context_window_fallbacks"], json!([{"c": ["d"]}]));
    assert_eq!(
        view["model_group_retry_policy"],
        json!({"gpt": {"DefaultRetries": 2}})
    );
    assert_eq!(view["model_group_alias"], json!({"x": ["y"]}));
    assert_eq!(view["enable_weighted_failover"], json!(true));
    assert_eq!(view["enable_tag_filtering"], json!(true));
    assert_eq!(view["tag_routing_prefix"], json!("tag:"));
    assert_eq!(view["max_retries"], json!(3.5));
}

#[rstest]
#[case::integer(json!(30), json!(30))]
#[case::negative_integer(json!(-4), json!(-4))]
#[case::float_truncated_down(json!(30.5), json!(30))]
#[case::float_truncated_toward_zero(json!(-2.9), json!(-2))]
#[case::float_below_one(json!(0.99), json!(0))]
#[case::string(json!("42"), json!(42))]
#[case::string_with_spaces(json!("  7\n"), json!(7))]
#[case::string_with_sign(json!("+8"), json!(8))]
#[case::integer_past_f64_precision(json!(9_007_199_254_740_993_i64), json!(9_007_199_254_740_993_i64))]
#[case::integer_past_i64(json!(18_446_744_073_709_551_615_u64), json!(18_446_744_073_709_551_615_u64))]
fn update_int_follows_python_int(#[case] given: Value, #[case] stored: Value) {
    let mut settings = router(full_values());

    update(&mut settings, json!({"timeout": given}));

    assert_eq!(shown(&settings)["timeout"], stored);
}

#[rstest]
#[case::timeout("timeout")]
#[case::num_retries("num_retries")]
#[case::retry_after("retry_after")]
#[case::allowed_fails("allowed_fails")]
#[case::cooldown_time("cooldown_time")]
fn update_int_casts_each_of_the_five_settings(#[case] name: &str) {
    let mut settings = router(full_values());

    update(&mut settings, json!({ name: 6 }));
    assert_eq!(shown(&settings)[name], json!(6));

    update(&mut settings, json!({ name: 1.9 }));
    assert_eq!(shown(&settings)[name], json!(1));
}

#[rstest]
#[case::null(Value::Null)]
#[case::boolean(json!(true))]
#[case::list(json!([1]))]
#[case::object(json!({"a": 1}))]
#[case::text(json!("abc"))]
#[case::empty_text(json!(""))]
#[case::decimal_text(json!("4.5"))]
#[case::float_past_i64(json!(1.0e30))]
fn update_int_refuses_what_python_int_refuses(#[case] given: Value) {
    let mut settings = router(full_values());
    let mut recorder = Recorder::default();

    let error = settings
        .update_settings(&mut recorder, &pairs(json!({"cooldown_time": given})))
        .unwrap_err();

    assert!(matches!(&error, RouterSettingsError::NotAnInteger { key } if key == "cooldown_time"));
    assert!(error.to_string().contains("cooldown_time"));
    assert_eq!(shown(&settings)["cooldown_time"], json!(5));
}

#[test]
fn update_int_failure_keeps_the_earlier_pairs_and_skips_the_later_ones() {
    let mut settings = router(full_values());
    let mut recorder = Recorder::default();

    let error = settings
        .update_settings(
            &mut recorder,
            &pairs(json!({"num_retries": 8, "timeout": "soon", "allowed_fails": 9})),
        )
        .unwrap_err();

    assert!(matches!(&error, RouterSettingsError::NotAnInteger { key } if key == "timeout"));
    let view = shown(&settings);
    assert_eq!(view["num_retries"], json!(8));
    assert_eq!(view["timeout"], json!(600));
    assert_eq!(view["allowed_fails"], json!(3));
}

#[test]
fn update_policy_object_is_stored_with_every_field() {
    let mut settings = router(full_values());

    update(
        &mut settings,
        json!({"retry_policy": {"RateLimitErrorRetries": 3, "DefaultRetries": null, "Other": "x"}}),
    );

    assert_eq!(
        shown(&settings)["retry_policy"],
        json!({
            "BadRequestErrorRetries": null,
            "AuthenticationErrorRetries": null,
            "TimeoutErrorRetries": null,
            "RateLimitErrorRetries": 3,
            "ContentPolicyViolationErrorRetries": null,
            "InternalServerErrorRetries": null,
            "ServiceUnavailableErrorRetries": null,
            "NotFoundErrorRetries": null,
            "DefaultRetries": null,
        })
    );
}

#[test]
fn update_policy_object_replaces_the_policy_instead_of_merging() {
    let mut settings = router(full_values());
    update(
        &mut settings,
        json!({"retry_policy": {"RateLimitErrorRetries": 3}}),
    );

    update(
        &mut settings,
        json!({"retry_policy": {"TimeoutErrorRetries": 1}}),
    );

    let policy = &shown(&settings)["retry_policy"];
    assert_eq!(policy["RateLimitErrorRetries"], Value::Null);
    assert_eq!(policy["TimeoutErrorRetries"], json!(1));
}

#[test]
fn update_policy_null_clears_the_policy() {
    let mut settings = router(full_values());
    update(
        &mut settings,
        json!({"retry_policy": {"RateLimitErrorRetries": 3}}),
    );

    update(&mut settings, json!({"retry_policy": null}));

    assert_eq!(shown(&settings)["retry_policy"], Value::Null);
}

#[rstest]
#[case::list(json!(["a"]))]
#[case::text(json!("x"))]
#[case::number(json!(3))]
#[case::boolean(json!(false))]
fn update_policy_other_values_keep_the_old_policy(#[case] given: Value) {
    let mut settings = router(full_values());
    update(
        &mut settings,
        json!({"retry_policy": {"RateLimitErrorRetries": 3}}),
    );
    let before = shown(&settings)["retry_policy"].clone();

    update(&mut settings, json!({"retry_policy": given}));

    assert_eq!(shown(&settings)["retry_policy"], before);
}

#[test]
fn update_policy_wrong_field_type_names_the_field_and_keeps_the_old_policy() {
    let mut settings = router(full_values());
    let mut recorder = Recorder::default();

    let error = settings
        .update_settings(
            &mut recorder,
            &pairs(json!({"retry_policy": {"NotFoundErrorRetries": "many"}})),
        )
        .unwrap_err();

    assert!(
        matches!(&error, RouterSettingsError::RetryPolicy { field, .. } if field == "NotFoundErrorRetries")
    );
    assert_eq!(shown(&settings)["retry_policy"], Value::Null);
}

#[test]
fn update_strategy_change_runs_the_init_with_its_args_and_rebuilds_the_groups() {
    let mut settings = router(full_values());

    let recorder = update(
        &mut settings,
        json!({
            "routing_strategy": "usage-based-routing-v2",
            "routing_strategy_args": {"ttl": 10},
        }),
    );

    assert_eq!(
        recorder.calls,
        [
            (
                "init_routing_strategy",
                json!({"strategy": "usage-based-routing-v2", "args": {"ttl": 10}})
            ),
            ("apply_updated_strategy_args", Value::Null),
            ("init_routing_groups", Value::Null),
        ]
    );
    assert_eq!(
        shown(&settings)["routing_strategy"],
        json!("usage-based-routing-v2")
    );
}

#[test]
fn update_strategy_init_gets_an_empty_object_when_no_args_come_with_it() {
    let mut settings = router(full_values());

    let recorder = update(&mut settings, json!({"routing_strategy": "least-busy"}));

    assert_eq!(
        recorder.calls[0],
        (
            "init_routing_strategy",
            json!({"strategy": "least-busy", "args": {}})
        )
    );
}

#[test]
fn update_strategy_init_reads_args_that_come_after_it() {
    let mut settings = router(full_values());
    let mut recorder = Recorder::default();
    let kwargs = vec![
        (
            "routing_strategy".to_owned(),
            json!("latency-based-routing"),
        ),
        (
            "routing_strategy_args".to_owned(),
            json!({"lowest_latency_buffer": 0.5}),
        ),
    ];

    settings.update_settings(&mut recorder, &kwargs).unwrap();

    assert_eq!(
        recorder.calls[0],
        (
            "init_routing_strategy",
            json!({"strategy": "latency-based-routing", "args": {"lowest_latency_buffer": 0.5}})
        )
    );
}

#[rstest]
#[case::with_args(json!({"routing_strategy": "lar1", "routing_strategy_args": {"a": 1}}), json!({"absent": false, "args": {"a": 1}}))]
#[case::without_args(json!({"routing_strategy": "lar1"}), json!({"absent": true, "args": null}))]
#[case::null_args(json!({"routing_strategy": "lar1", "routing_strategy_args": null}), json!({"absent": true, "args": null}))]
fn update_strategy_lar1_gets_the_args_pair_or_none(
    #[case] kwargs: Value,
    #[case] expected_args: Value,
) {
    let mut settings = router(full_values());

    let recorder = update(&mut settings, kwargs);

    assert_eq!(recorder.calls[0], ("apply_lar1", expected_args));
    assert!(
        recorder
            .calls
            .iter()
            .all(|(step, _)| *step != "init_routing_strategy")
    );
    assert_eq!(shown(&settings)["routing_strategy"], json!("lar1"));
}

#[test]
fn update_strategy_same_as_before_changes_nothing_and_leaves_the_groups_alone() {
    let mut settings = router(full_values());

    let recorder = update(&mut settings, json!({"routing_strategy": "simple-shuffle"}));

    assert!(recorder.calls.is_empty());
}

#[test]
fn update_strategy_is_compared_with_the_value_from_before_the_loop() {
    let mut settings = router(full_values());
    let mut recorder = Recorder::default();
    let kwargs = vec![
        ("routing_strategy".to_owned(), json!("least-busy")),
        ("routing_strategy".to_owned(), json!("least-busy")),
    ];

    settings.update_settings(&mut recorder, &kwargs).unwrap();

    let inits = recorder
        .calls
        .iter()
        .filter(|(step, _)| *step == "init_routing_strategy")
        .count();
    assert_eq!(inits, 2);
}

#[test]
fn update_strategy_failed_init_does_not_store_the_strategy() {
    let mut settings = router(full_values());
    let mut recorder = Recorder {
        calls: Vec::new(),
        fail_groups: false,
        fail_strategy: true,
    };

    let error = settings
        .update_settings(
            &mut recorder,
            &pairs(json!({"num_retries": 7, "routing_strategy": "nonsense", "timeout": 1})),
        )
        .unwrap_err();

    assert!(matches!(error, RouterSettingsError::Target { .. }));
    let view = shown(&settings);
    assert_eq!(view["routing_strategy"], json!("simple-shuffle"));
    assert_eq!(view["num_retries"], json!(7));
    assert_eq!(view["timeout"], json!(600));
}

#[rstest]
#[case::changed(json!({"a": 2}), true)]
#[case::equal(json!({}), false)]
fn update_strategy_args_step_runs_once_after_the_loop_when_the_args_changed(
    #[case] args: Value,
    #[case] runs: bool,
) {
    let mut settings = router(full_values());

    let recorder = update(
        &mut settings,
        json!({"routing_strategy_args": args, "timeout": 5}),
    );

    let steps: Vec<&str> = recorder.calls.iter().map(|(step, _)| *step).collect();
    assert_eq!(steps == ["apply_updated_strategy_args"], runs);
    assert_eq!(steps.is_empty(), !runs);
    assert_eq!(shown(&settings)["routing_strategy_args"], args);
}

#[test]
fn update_strategy_pre_call_checks_run_in_place_and_are_not_stored() {
    let mut settings = router(full_values());

    let recorder = update(
        &mut settings,
        json!({"optional_pre_call_checks": ["prompt_caching"], "routing_strategy": "least-busy"}),
    );

    assert_eq!(
        recorder.calls[0],
        ("set_optional_pre_call_checks", json!(["prompt_caching"]))
    );
    assert_eq!(recorder.calls.len(), 3);
    assert!(!shown(&settings).contains_key("optional_pre_call_checks"));
    assert!(!settings.values.contains_key("optional_pre_call_checks"));
}

#[test]
fn update_groups_pair_rebuilds_with_the_new_input_and_stores_it() {
    let mut settings = router(full_values());
    let groups = json!([{"name": "fast", "routing_strategy": "least-busy"}]);

    let recorder = update(&mut settings, json!({"routing_groups": groups}));

    assert_eq!(recorder.calls, [("init_routing_groups", groups.clone())]);
    assert_eq!(settings.routing_groups_input, Some(groups));
    assert!(!settings.values.contains_key("routing_groups"));
}

#[test]
fn update_groups_rebuild_after_a_strategy_change_uses_the_stored_input() {
    let stored = json!([{"name": "kept"}]);
    let mut settings = router(full_values());
    settings.routing_groups_input = Some(stored.clone());

    let recorder = update(&mut settings, json!({"routing_strategy": "least-busy"}));

    assert_eq!(
        recorder.calls.last().unwrap(),
        &("init_routing_groups", stored.clone())
    );
    assert_eq!(settings.routing_groups_input, Some(stored));
}

#[test]
fn update_groups_pair_wins_over_the_stored_input_on_a_strategy_change() {
    let mut settings = router(full_values());
    settings.routing_groups_input = Some(json!([{"name": "old"}]));

    let recorder = update(
        &mut settings,
        json!({"routing_strategy": "least-busy", "routing_groups": [{"name": "new"}]}),
    );

    assert_eq!(
        recorder.calls.last().unwrap(),
        &("init_routing_groups", json!([{"name": "new"}]))
    );
    assert_eq!(
        settings.routing_groups_input,
        Some(json!([{"name": "new"}]))
    );
}

#[test]
fn update_groups_null_pair_rebuilds_with_no_groups() {
    let mut settings = router(full_values());
    settings.routing_groups_input = Some(json!([{"name": "old"}]));

    let recorder = update(&mut settings, json!({"routing_groups": null}));

    assert_eq!(recorder.calls, [("init_routing_groups", Value::Null)]);
    assert_eq!(settings.routing_groups_input, None);
}

#[test]
fn update_groups_are_left_alone_when_nothing_asks_for_a_rebuild() {
    let mut settings = router(full_values());
    settings.routing_groups_input = Some(json!([{"name": "old"}]));

    let recorder = update(&mut settings, json!({"timeout": 3}));

    assert!(recorder.calls.is_empty());
    assert_eq!(
        settings.routing_groups_input,
        Some(json!([{"name": "old"}]))
    );
}

#[test]
fn update_groups_failed_rebuild_keeps_the_old_input_and_every_earlier_pair() {
    let old = json!([{"name": "old"}]);
    let mut settings = router(full_values());
    settings.routing_groups_input = Some(old.clone());
    let mut recorder = Recorder {
        calls: Vec::new(),
        fail_groups: true,
        fail_strategy: false,
    };

    let error = settings
        .update_settings(
            &mut recorder,
            &pairs(json!({"routing_groups": "bad", "num_retries": 11})),
        )
        .unwrap_err();

    assert!(
        matches!(&error, RouterSettingsError::Target { name, message } if name == "init_routing_groups" && message == "invalid routing_groups")
    );
    assert_eq!(settings.routing_groups_input, Some(old));
    assert_eq!(shown(&settings)["num_retries"], json!(11));
}
