use super::support::{
    RecordingSinks, TENANT_A, allowed_hosts, attach, build_adapter, build_key_with_disable_list,
    build_key_with_team, build_runtime, disable_header, langfuse_entry, logging, newrelic_entry,
    no_headers, seeded_names, setup,
};

#[tokio::test]
async fn callback_disabling() {
    let runtime = build_runtime();
    let sinks = RecordingSinks::new();
    let _fan_out = attach(&runtime, &sinks);
    let entries = [
        langfuse_entry(TENANT_A, "pk-a", "sk-a"),
        newrelic_entry("nr-a"),
    ]
    .join(",");
    let listed = build_key_with_disable_list(r#""langfuse_otel""#, &entries);
    let adapter = build_adapter(&runtime, setup(allowed_hosts(&[TENANT_A])));
    let header = disable_header("newrelic");
    let from_key_list = seeded_names(&runtime, &adapter, &listed, &no_headers()).await;
    assert_eq!(from_key_list, vec!["newrelic".to_owned()]);
    let from_header = seeded_names(&runtime, &adapter, &listed, &header).await;
    assert_eq!(from_header, vec!["langfuse_otel".to_owned()]);
    let team = r#"{"litellm_disabled_callbacks": ["langfuse_otel"]}"#;
    let keyed = build_key_with_team(&logging(&langfuse_entry(TENANT_A, "pk-a", "sk-a")), team);
    let from_team = seeded_names(&runtime, &adapter, &keyed, &no_headers()).await;
    assert_eq!(from_team, vec!["langfuse_otel".to_owned()]);
    let mut plain = setup(allowed_hosts(&[TENANT_A]));
    plain.callback_disabling = false;
    let unswitched = build_adapter(&runtime, plain);
    let switch_off = seeded_names(&runtime, &unswitched, &listed, &header).await;
    assert_eq!(
        switch_off,
        vec!["langfuse_otel".to_owned(), "newrelic".to_owned()]
    );
}
