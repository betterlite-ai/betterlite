#[rstest]
#[case::one("1", true)]
#[case::true_word("true", true)]
#[case::yes("yes", true)]
#[case::on("on", true)]
#[case::padded_mixed_case(" On ", true)]
#[case::upper_case("TRUE", true)]
#[case::empty("", false)]
#[case::blank("   ", false)]
#[case::zero("0", false)]
#[case::false_word("false", false)]
#[case::off("off", false)]
fn settings_on_startup_reads_the_shared_flag_values(#[case] value: &str, #[case] expected: bool) {
    let environment = BTreeMap::from([(String::from(ON_STARTUP_VARIABLE), value.to_owned())]);
    assert_eq!(
        DiscoverySettings::from_lookup(&environment)
            .unwrap()
            .on_startup,
        expected
    );
}

#[test]
fn settings_on_startup_rejects_a_word_that_is_not_a_bool() {
    let environment =
        BTreeMap::from([(String::from(ON_STARTUP_VARIABLE), String::from("enabled"))]);
    assert_eq!(
        DiscoverySettings::from_lookup(&environment),
        Err(EnvError::Bool {
            name: String::from(ON_STARTUP_VARIABLE)
        })
    );
}

#[test]
fn settings_defaults_are_litellm_numbers() {
    assert_eq!(
        DiscoverySettings::default(),
        settings_with(false, litellm_delays())
    );
}

#[test]
fn settings_absent_variables_give_the_defaults() {
    let environment = BTreeMap::<String, String>::new();
    assert_eq!(
        DiscoverySettings::from_lookup(&environment),
        Ok(settings_with(false, litellm_delays()))
    );
}

#[test]
fn settings_lookup_keeps_the_default_numbers_and_reads_the_fetch_timeout() {
    let environment = BTreeMap::from([
        (String::from(ON_STARTUP_VARIABLE), String::from("yes")),
        (String::from(METADATA_TIMEOUT_VARIABLE), String::from("2")),
    ]);
    let expected = DiscoverySettings {
        on_startup: true,
        retry_delays: litellm_delays(),
        backoff_base: Duration::from_secs(30),
        backoff_cap: Duration::from_secs(900),
        temporary_ttl: Duration::from_secs(300),
        fetch: FetchSettings {
            metadata_timeout: Duration::from_secs(2),
        },
    };
    assert_eq!(DiscoverySettings::from_lookup(&environment), Ok(expected));
}

#[rstest]
#[case::no_failure(0, 30)]
#[case::first(1, 30)]
#[case::second(2, 60)]
#[case::third(3, 120)]
#[case::fifth(5, 480)]
#[case::sixth(6, 900)]
#[case::seventh(7, 900)]
#[case::shift_past_the_width(33, 900)]
#[case::saturated(u32::MAX, 900)]
fn settings_backoff_doubles_from_thirty_seconds_up_to_the_cap(
    #[case] consecutive_failures: u32,
    #[case] seconds: u64,
) {
    assert_eq!(
        DiscoverySettings::default().backoff(consecutive_failures),
        Duration::from_secs(seconds)
    );
}

#[test]
fn settings_backoff_follows_shortened_numbers() {
    let settings = DiscoverySettings {
        on_startup: false,
        retry_delays: Vec::new(),
        backoff_base: Duration::from_secs(1),
        backoff_cap: Duration::from_secs(5),
        temporary_ttl: Duration::from_secs(1),
        fetch: FetchSettings::default(),
    };
    assert_eq!(settings.backoff(3), Duration::from_secs(4));
    assert_eq!(settings.backoff(4), Duration::from_secs(5));
}
