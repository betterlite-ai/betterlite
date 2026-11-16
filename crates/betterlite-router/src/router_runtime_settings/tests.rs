#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;
use serde_json::json;

use super::error::RouterSettingsError;
use super::target::RouterSettingsTarget;
use super::{RouterRuntimeSettings, SettingsView, UPDATABLE_ROUTER_SETTINGS};

type Calls = Vec<(&'static str, JsonValue)>;

#[derive(Default)]
struct Recorder {
    calls: Calls,
    fail_groups: bool,
    fail_strategy: bool,
}

type Step = Result<(), RouterSettingsError>;

fn failure(name: &str, message: &str) -> Step {
    Err(RouterSettingsError::Target {
        name: name.to_owned(),
        message: message.to_owned(),
    })
}

impl RouterSettingsTarget for Recorder {
    fn set_optional_pre_call_checks(&mut self, checks: &JsonValue) -> Step {
        self.calls
            .push(("set_optional_pre_call_checks", checks.clone()));
        Ok(())
    }

    fn init_routing_strategy(&mut self, strategy: &JsonValue, args: &JsonValue) -> Step {
        let call = object(vec![("strategy", strategy.clone()), ("args", args.clone())]);
        self.calls.push(("init_routing_strategy", call));
        if self.fail_strategy {
            return failure("init_routing_strategy", "unknown strategy");
        }
        Ok(())
    }

    fn apply_updated_strategy_args(&mut self) -> Step {
        self.calls
            .push(("apply_updated_strategy_args", JsonValue::Null));
        Ok(())
    }

    fn apply_lar1(&mut self, args: Option<&JsonValue>) -> Step {
        let call = object(vec![
            ("absent", JsonValue::Bool(args.is_none())),
            ("args", args.cloned().unwrap_or(JsonValue::Null)),
        ]);
        self.calls.push(("apply_lar1", call));
        Ok(())
    }

    fn init_routing_groups(&mut self, input: Option<&JsonValue>) -> Step {
        self.calls.push((
            "init_routing_groups",
            input.cloned().unwrap_or(JsonValue::Null),
        ));
        if self.fail_groups {
            return failure("init_routing_groups", "invalid routing_groups");
        }
        Ok(())
    }
}

fn router(values: JsonValue) -> RouterRuntimeSettings {
    let JsonValue::Object(fields) = values else {
        panic!("router settings values must be an object");
    };
    RouterRuntimeSettings {
        values: fields.into_iter().collect(),
        routing_groups_input: None,
    }
}

fn pairs(entries: Vec<(&str, JsonValue)>) -> Vec<(String, JsonValue)> {
    entries
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

// json!({"a": 1}) expands to serde_json's Map and Into::into, which this crate denies
fn object(entries: Vec<(&str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
    )
}

fn update(router: &mut RouterRuntimeSettings, entries: Vec<(&str, JsonValue)>) -> Recorder {
    let mut recorder = Recorder::default();
    router
        .update_settings(&mut recorder, &pairs(entries))
        .unwrap();
    recorder
}

fn shown(router: &RouterRuntimeSettings) -> BTreeMap<String, JsonValue> {
    router.get_settings(&SettingsView::default())
}

fn full_values() -> JsonValue {
    object(vec![
        ("routing_strategy_args", object(vec![])),
        ("routing_strategy", json!("simple-shuffle")),
        ("allowed_fails", json!(3_i64)),
        ("cooldown_time", json!(5_i64)),
        ("num_retries", json!(2_i64)),
        ("timeout", json!(600_i64)),
        ("retry_after", json!(0_i64)),
        (
            "fallbacks",
            json!([object(vec![("gpt", json!(["claude"]))])]),
        ),
        ("context_window_fallbacks", json!([])),
        ("model_group_retry_policy", JsonValue::Null),
        ("retry_policy", JsonValue::Null),
        ("model_group_alias", object(vec![])),
        ("enable_weighted_failover", json!(false)),
        ("enable_tag_filtering", json!(false)),
        ("tag_routing_prefix", json!("")),
    ])
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
    assert_eq!(settings["timeout"], json!(600_i64));
    assert_eq!(settings["routing_groups"], json!([]));
}

#[test]
fn get_settings_skips_a_name_the_router_never_stored() {
    let settings =
        router(object(vec![("timeout", json!(5_i64))])).get_settings(&SettingsView::default());

    let mut names: Vec<&str> = settings.keys().map(String::as_str).collect();
    names.sort_unstable();
    assert_eq!(names, ["routing_groups", "timeout"]);
}

#[test]
fn get_settings_hides_values_outside_the_list() {
    let mut values = full_values();
    values["optional_pre_call_checks"] = json!(["prompt_caching"]);
    values["routing_groups"] = json!([object(vec![("name", json!("stored"))])]);

    let settings = router(values).get_settings(&SettingsView::default());

    assert!(!settings.contains_key("optional_pre_call_checks"));
    assert_eq!(settings["routing_groups"], json!([]));
}

#[test]
fn get_settings_adds_the_dumped_routing_groups() {
    let view = SettingsView {
        routing_groups: vec![
            object(vec![("name", json!("fast"))]),
            object(vec![("name", json!("cheap"))]),
        ],
        latency_routing_args: None,
    };

    let settings = router(full_values()).get_settings(&view);

    assert_eq!(
        settings["routing_groups"],
        json!([
            object(vec![("name", json!("fast"))]),
            object(vec![("name", json!("cheap"))]),
        ])
    );
}

#[rstest]
#[case::latency_with_args("latency-based-routing", Some("{\"ttl\":3600}"), json!("{\"ttl\":3600}"))]
#[case::latency_without_args(
    "latency-based-routing",
    None,
    object(vec![("ttl", json!(1_i64))])
)]
#[case::other_strategy_keeps_the_object(
    "usage-based-routing",
    Some("{\"ttl\":3600}"),
    object(vec![("ttl", json!(1_i64))])
)]
fn get_settings_shows_latency_args_as_text_only_for_the_latency_strategy(
    #[case] strategy: &str,
    #[case] latency_args: Option<&str>,
    #[case] expected: JsonValue,
) {
    let mut values = full_values();
    values["routing_strategy"] = json!(strategy);
    values["routing_strategy_args"] = object(vec![("ttl", json!(1_i64))]);
    let view = SettingsView {
        routing_groups: vec![],
        latency_routing_args: latency_args.map(str::to_owned),
    };

    let settings = router(values).get_settings(&view);

    assert_eq!(settings["routing_strategy_args"], expected);
    assert_eq!(settings["timeout"], json!(600_i64));
}

#[test]
fn get_settings_shows_max_retries_once_an_update_sets_it() {
    let mut settings = router(full_values());
    assert!(!shown(&settings).contains_key("max_retries"));

    update(&mut settings, vec![("max_retries", json!(4_i64))]);

    assert_eq!(shown(&settings)["max_retries"], json!(4_i64));
}

#[test]
fn get_settings_never_shows_a_name_update_skipped() {
    let mut settings = router(full_values());

    let recorder = update(
        &mut settings,
        vec![("cache", json!(true)), ("num_retries", json!(9_i64))],
    );

    assert!(recorder.calls.is_empty());
    assert!(!shown(&settings).contains_key("cache"));
    assert_eq!(shown(&settings)["num_retries"], json!(9_i64));
}

#[test]
fn get_settings_names_every_updatable_setting_once() {
    let mut names = UPDATABLE_ROUTER_SETTINGS.to_vec();
    names.sort_unstable();
    names.dedup();

    assert_eq!(names.len(), 18);
}

#[test]
fn get_settings_stores_the_other_names_as_given() {
    let mut settings = router(full_values());

    update(
        &mut settings,
        vec![
            ("fallbacks", json!([object(vec![("a", json!(["b"]))])])),
            (
                "context_window_fallbacks",
                json!([object(vec![("c", json!(["d"]))])]),
            ),
            (
                "model_group_retry_policy",
                object(vec![(
                    "gpt",
                    object(vec![("DefaultRetries", json!(2_i64))]),
                )]),
            ),
            ("model_group_alias", object(vec![("x", json!(["y"]))])),
            ("enable_weighted_failover", json!(true)),
            ("enable_tag_filtering", json!(true)),
            ("tag_routing_prefix", json!("tag:")),
            ("max_retries", json!(3.5_f64)),
        ],
    );

    let view = shown(&settings);
    assert_eq!(
        view["fallbacks"],
        json!([object(vec![("a", json!(["b"]))])])
    );
    assert_eq!(
        view["context_window_fallbacks"],
        json!([object(vec![("c", json!(["d"]))])])
    );
    assert_eq!(
        view["model_group_retry_policy"],
        object(vec![(
            "gpt",
            object(vec![("DefaultRetries", json!(2_i64))])
        )])
    );
    assert_eq!(view["model_group_alias"], object(vec![("x", json!(["y"]))]));
    assert_eq!(view["enable_weighted_failover"], json!(true));
    assert_eq!(view["enable_tag_filtering"], json!(true));
    assert_eq!(view["tag_routing_prefix"], json!("tag:"));
    assert_eq!(view["max_retries"], json!(3.5_f64));
}

#[rstest]
#[case::integer(json!(30_i64), json!(30_i64))]
#[case::negative_integer(json!(-4_i64), json!(-4_i64))]
#[case::float_truncated_down(json!(30.5_f64), json!(30_i64))]
#[case::float_truncated_toward_zero(json!(-2.9_f64), json!(-2_i64))]
#[case::float_below_one(json!(0.99_f64), json!(0_i64))]
#[case::string(json!("42"), json!(42_i64))]
#[case::string_with_spaces(json!("  7\n"), json!(7_i64))]
#[case::string_with_sign(json!("+8"), json!(8_i64))]
#[case::float_past_i64_into_u64(json!(1.0e19_f64), json!(10_u64.pow(19)))]
#[case::integer_past_f64_precision(json!(0x0020_0000_0000_0001_i64), json!(0x0020_0000_0000_0001_i64))]
#[case::integer_past_i64(json!(0xffff_ffff_ffff_ffff_u64), json!(0xffff_ffff_ffff_ffff_u64))]
fn update_int_follows_python_int(#[case] given: JsonValue, #[case] stored: JsonValue) {
    let mut settings = router(full_values());

    update(&mut settings, vec![("timeout", given)]);

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

    update(&mut settings, vec![(name, json!(6_i64))]);
    assert_eq!(shown(&settings)[name], json!(6_i64));

    update(&mut settings, vec![(name, json!(1.9_f64))]);
    assert_eq!(shown(&settings)[name], json!(1_i64));
}

#[rstest]
#[case::null(JsonValue::Null)]
#[case::boolean(json!(true))]
#[case::list(json!([1_i64]))]
#[case::object(object(vec![("a", json!(1_i64))]))]
#[case::text(json!("abc"))]
#[case::empty_text(json!(""))]
#[case::decimal_text(json!("4.5"))]
// the last two are past the stored range, which docs/deviations/B9711.yaml lists
#[case::float_past_decimal_range(json!(1.0e30_f64))]
#[case::float_below_i64(json!(-1.0e19_f64))]
fn update_int_refuses_what_it_cannot_store(#[case] given: JsonValue) {
    let mut settings = router(full_values());
    let mut recorder = Recorder::default();

    let error = settings
        .update_settings(&mut recorder, &pairs(vec![("cooldown_time", given)]))
        .unwrap_err();

    assert!(matches!(&error, RouterSettingsError::NotAnInteger { key } if key == "cooldown_time"));
    assert!(error.to_string().contains("cooldown_time"));
    assert_eq!(shown(&settings)["cooldown_time"], json!(5_i64));
}

#[test]
fn update_int_failure_keeps_the_earlier_pairs_and_skips_the_later_ones() {
    let mut settings = router(full_values());
    let mut recorder = Recorder::default();

    let error = settings
        .update_settings(
            &mut recorder,
            &pairs(vec![
                ("num_retries", json!(8_i64)),
                ("timeout", json!("soon")),
                ("allowed_fails", json!(9_i64)),
            ]),
        )
        .unwrap_err();

    assert!(matches!(&error, RouterSettingsError::NotAnInteger { key } if key == "timeout"));
    let view = shown(&settings);
    assert_eq!(view["num_retries"], json!(8_i64));
    assert_eq!(view["timeout"], json!(600_i64));
    assert_eq!(view["allowed_fails"], json!(3_i64));
}

#[test]
fn update_policy_object_is_stored_with_every_field() {
    let mut settings = router(full_values());

    update(
        &mut settings,
        vec![(
            "retry_policy",
            object(vec![
                ("RateLimitErrorRetries", json!(3_i64)),
                ("DefaultRetries", JsonValue::Null),
                ("Other", json!("x")),
            ]),
        )],
    );

    assert_eq!(
        shown(&settings)["retry_policy"],
        object(vec![
            ("BadRequestErrorRetries", JsonValue::Null),
            ("AuthenticationErrorRetries", JsonValue::Null),
            ("TimeoutErrorRetries", JsonValue::Null),
            ("RateLimitErrorRetries", json!(3_i64)),
            ("ContentPolicyViolationErrorRetries", JsonValue::Null),
            ("InternalServerErrorRetries", JsonValue::Null),
            ("ServiceUnavailableErrorRetries", JsonValue::Null),
            ("NotFoundErrorRetries", JsonValue::Null),
            ("DefaultRetries", JsonValue::Null),
        ])
    );
}

#[test]
fn update_policy_object_replaces_the_policy_instead_of_merging() {
    let mut settings = router(full_values());
    update(
        &mut settings,
        vec![(
            "retry_policy",
            object(vec![("RateLimitErrorRetries", json!(3_i64))]),
        )],
    );

    update(
        &mut settings,
        vec![(
            "retry_policy",
            object(vec![("TimeoutErrorRetries", json!(1_i64))]),
        )],
    );

    let policy = &shown(&settings)["retry_policy"];
    assert_eq!(policy["RateLimitErrorRetries"], JsonValue::Null);
    assert_eq!(policy["TimeoutErrorRetries"], json!(1_i64));
}

#[test]
fn update_policy_null_clears_the_policy() {
    let mut settings = router(full_values());
    update(
        &mut settings,
        vec![(
            "retry_policy",
            object(vec![("RateLimitErrorRetries", json!(3_i64))]),
        )],
    );

    update(&mut settings, vec![("retry_policy", JsonValue::Null)]);

    assert_eq!(shown(&settings)["retry_policy"], JsonValue::Null);
}

#[rstest]
#[case::list(json!(["a"]))]
#[case::text(json!("x"))]
#[case::number(json!(3_i64))]
#[case::boolean(json!(false))]
fn update_policy_other_values_keep_the_old_policy(#[case] given: JsonValue) {
    let mut settings = router(full_values());
    update(
        &mut settings,
        vec![(
            "retry_policy",
            object(vec![("RateLimitErrorRetries", json!(3_i64))]),
        )],
    );
    let before = shown(&settings)["retry_policy"].clone();

    update(&mut settings, vec![("retry_policy", given)]);

    assert_eq!(shown(&settings)["retry_policy"], before);
}

#[test]
fn update_policy_wrong_field_type_names_the_field_and_keeps_the_old_policy() {
    let mut settings = router(full_values());
    update(
        &mut settings,
        vec![(
            "retry_policy",
            object(vec![("RateLimitErrorRetries", json!(3_i64))]),
        )],
    );
    let before = shown(&settings)["retry_policy"].clone();
    let mut recorder = Recorder::default();

    let error = settings
        .update_settings(
            &mut recorder,
            &pairs(vec![(
                "retry_policy",
                object(vec![("NotFoundErrorRetries", json!("many"))]),
            )]),
        )
        .unwrap_err();

    assert!(
        matches!(&error, RouterSettingsError::RetryPolicy { field, .. } if field == "NotFoundErrorRetries")
    );
    assert_eq!(shown(&settings)["retry_policy"], before);
    assert_eq!(before["RateLimitErrorRetries"], json!(3_i64));
}

#[test]
fn update_strategy_change_runs_the_init_with_its_args_and_rebuilds_the_groups() {
    let mut settings = router(full_values());

    let recorder = update(
        &mut settings,
        vec![
            ("routing_strategy", json!("usage-based-routing-v2")),
            (
                "routing_strategy_args",
                object(vec![("ttl", json!(10_i64))]),
            ),
        ],
    );

    assert_eq!(
        recorder.calls,
        [
            (
                "init_routing_strategy",
                object(vec![
                    ("strategy", json!("usage-based-routing-v2")),
                    ("args", object(vec![("ttl", json!(10_i64))])),
                ])
            ),
            ("apply_updated_strategy_args", JsonValue::Null),
            ("init_routing_groups", JsonValue::Null),
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

    let recorder = update(
        &mut settings,
        vec![("routing_strategy", json!("least-busy"))],
    );

    assert_eq!(
        recorder.calls[0],
        (
            "init_routing_strategy",
            object(vec![
                ("strategy", json!("least-busy")),
                ("args", object(vec![])),
            ])
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
            object(vec![("lowest_latency_buffer", json!(0.5_f64))]),
        ),
    ];

    settings.update_settings(&mut recorder, &kwargs).unwrap();

    assert_eq!(
        recorder.calls[0],
        (
            "init_routing_strategy",
            object(vec![
                ("strategy", json!("latency-based-routing")),
                (
                    "args",
                    object(vec![("lowest_latency_buffer", json!(0.5_f64))]),
                ),
            ])
        )
    );
}

#[rstest]
#[case::with_args(
    vec![
        ("routing_strategy", json!("lar1")),
        ("routing_strategy_args", object(vec![("a", json!(1_i64))])),
    ],
    object(vec![
        ("absent", json!(false)),
        ("args", object(vec![("a", json!(1_i64))])),
    ])
)]
#[case::without_args(
    vec![("routing_strategy", json!("lar1"))],
    object(vec![("absent", json!(true)), ("args", JsonValue::Null)])
)]
#[case::null_args(
    vec![
        ("routing_strategy", json!("lar1")),
        ("routing_strategy_args", JsonValue::Null),
    ],
    object(vec![("absent", json!(true)), ("args", JsonValue::Null)])
)]
fn update_strategy_lar1_gets_the_args_pair_or_none(
    #[case] kwargs: Vec<(&str, JsonValue)>,
    #[case] expected_args: JsonValue,
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

    let recorder = update(
        &mut settings,
        vec![("routing_strategy", json!("simple-shuffle"))],
    );

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
            &pairs(vec![
                ("num_retries", json!(7_i64)),
                ("routing_strategy", json!("nonsense")),
                ("timeout", json!(1_i64)),
            ]),
        )
        .unwrap_err();

    assert!(matches!(error, RouterSettingsError::Target { .. }));
    let view = shown(&settings);
    assert_eq!(view["routing_strategy"], json!("simple-shuffle"));
    assert_eq!(view["num_retries"], json!(7_i64));
    assert_eq!(view["timeout"], json!(600_i64));
}

#[rstest]
#[case::changed(object(vec![("a", json!(2_i64))]), true)]
#[case::equal(object(vec![]), false)]
fn update_strategy_args_step_runs_once_after_the_loop_when_the_args_changed(
    #[case] args: JsonValue,
    #[case] runs: bool,
) {
    let mut settings = router(full_values());

    let recorder = update(
        &mut settings,
        vec![
            ("routing_strategy_args", args.clone()),
            ("timeout", json!(5_i64)),
        ],
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
        vec![
            ("optional_pre_call_checks", json!(["prompt_caching"])),
            ("routing_strategy", json!("least-busy")),
        ],
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
    let groups = json!([object(vec![
        ("name", json!("fast")),
        ("routing_strategy", json!("least-busy")),
    ])]);

    let recorder = update(&mut settings, vec![("routing_groups", groups.clone())]);

    assert_eq!(recorder.calls, [("init_routing_groups", groups.clone())]);
    assert_eq!(settings.routing_groups_input, Some(groups));
    assert!(!settings.values.contains_key("routing_groups"));
}

#[test]
fn update_groups_rebuild_after_a_strategy_change_uses_the_stored_input() {
    let stored = json!([object(vec![("name", json!("kept"))])]);
    let mut settings = router(full_values());
    settings.routing_groups_input = Some(stored.clone());

    let recorder = update(
        &mut settings,
        vec![("routing_strategy", json!("least-busy"))],
    );

    assert_eq!(
        recorder.calls.last().unwrap(),
        &("init_routing_groups", stored.clone())
    );
    assert_eq!(settings.routing_groups_input, Some(stored));
}

#[test]
fn update_groups_pair_wins_over_the_stored_input_on_a_strategy_change() {
    let mut settings = router(full_values());
    settings.routing_groups_input = Some(json!([object(vec![("name", json!("old"))])]));

    let recorder = update(
        &mut settings,
        vec![
            ("routing_strategy", json!("least-busy")),
            (
                "routing_groups",
                json!([object(vec![("name", json!("new"))])]),
            ),
        ],
    );

    assert_eq!(
        recorder.calls.last().unwrap(),
        &(
            "init_routing_groups",
            json!([object(vec![("name", json!("new"))])])
        )
    );
    assert_eq!(
        settings.routing_groups_input,
        Some(json!([object(vec![("name", json!("new"))])]))
    );
}

#[test]
fn update_groups_null_pair_rebuilds_with_no_groups() {
    let mut settings = router(full_values());
    settings.routing_groups_input = Some(json!([object(vec![("name", json!("old"))])]));

    let recorder = update(&mut settings, vec![("routing_groups", JsonValue::Null)]);

    assert_eq!(recorder.calls, [("init_routing_groups", JsonValue::Null)]);
    assert_eq!(settings.routing_groups_input, None);
}

#[test]
fn update_groups_are_left_alone_when_nothing_asks_for_a_rebuild() {
    let mut settings = router(full_values());
    settings.routing_groups_input = Some(json!([object(vec![("name", json!("old"))])]));

    let recorder = update(&mut settings, vec![("timeout", json!(3_i64))]);

    assert!(recorder.calls.is_empty());
    assert_eq!(
        settings.routing_groups_input,
        Some(json!([object(vec![("name", json!("old"))])]))
    );
}

#[test]
fn update_groups_failed_rebuild_keeps_the_old_input_and_every_earlier_pair() {
    let old = json!([object(vec![("name", json!("old"))])]);
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
            &pairs(vec![
                ("num_retries", json!(11_i64)),
                ("routing_groups", json!("bad")),
            ]),
        )
        .unwrap_err();

    assert!(
        matches!(&error, RouterSettingsError::Target { name, message } if name == "init_routing_groups" && message == "invalid routing_groups")
    );
    assert_eq!(settings.routing_groups_input, Some(old));
    assert_eq!(shown(&settings)["num_retries"], json!(11_i64));
}
