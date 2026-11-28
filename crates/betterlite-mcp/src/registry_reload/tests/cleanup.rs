#[tokio::test]
async fn cleanup_removes_openapi_tools_by_prefix_and_owned_mapping_rows() {
    let fixture = test_fixture();
    let subject = server(
        r#"{"server_id":"srv-1","name":"Server One","transport":"http","spec_path":"/spec.json","alias":"Prefix One"}"#,
    );
    fixture
        .tools
        .seed_tools(&["Prefix_One-echo", "Prefix_One", "Other-echo"]);
    {
        let mut owners = fixture.owners.write().unwrap();
        owners
            .mapping
            .insert(String::from("echo"), String::from("Prefix One"));
        owners
            .mapping
            .insert(String::from("Prefix_One-echo"), String::from("Prefix One"));
        owners
            .mapping
            .insert(String::from("bare"), String::from("Prefix_One"));
        owners
            .mapping
            .insert(String::from("named"), String::from("Server One"));
        owners
            .mapping
            .insert(String::from("other"), String::from("Other"));
    }
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.cleanup_tool_routing_artifacts(&subject);
    assert_eq!(fixture.tools.unregistered(), strings(&["Prefix_One-"]));
    let tools = fixture.tools.tools();
    assert!(tools.contains("Prefix_One"));
    assert!(tools.contains("Other-echo"));
    assert!(!tools.contains("Prefix_One-echo"));
    assert_eq!(fixture.caches.invalidated(), strings(&["srv-1"]));
    let mapping = fixture.owners.read().unwrap().mapping.clone();
    assert_eq!(
        mapping.keys().cloned().collect::<Vec<String>>(),
        strings(&["other"])
    );
}

#[tokio::test]
async fn cleanup_skips_unregister_without_spec_path() {
    let fixture = test_fixture();
    let subject = server(
        r#"{"server_id":"srv-1","name":"Server One","transport":"http","alias":"Prefix One"}"#,
    );
    fixture.tools.seed_tools(&["Prefix_One-echo"]);
    {
        let mut owners = fixture.owners.write().unwrap();
        owners
            .mapping
            .insert(String::from("echo"), String::from("Prefix One"));
    }
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.cleanup_tool_routing_artifacts(&subject);
    assert!(fixture.tools.unregistered().is_empty());
    assert!(fixture.tools.tools().contains("Prefix_One-echo"));
    assert!(fixture.owners.read().unwrap().mapping.is_empty());
    assert_eq!(fixture.caches.invalidated(), strings(&["srv-1"]));
}

#[tokio::test]
async fn cleanup_skips_unregister_without_a_prefix() {
    let fixture = test_fixture();
    let mut subject =
        server(r#"{"server_id":"tmp","name":"tmp","transport":"http","spec_path":"/spec.json"}"#);
    subject.fields.server_id = String::new();
    subject.fields.name = String::new();
    fixture.tools.seed_tools(&["anything"]);
    {
        let mut owners = fixture.owners.write().unwrap();
        owners.mapping.insert(String::from("blank"), String::new());
        owners
            .mapping
            .insert(String::from("echo"), String::from("Some Server"));
    }
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    mutator.cleanup_tool_routing_artifacts(&subject);
    assert!(fixture.tools.unregistered().is_empty());
    assert!(fixture.tools.tools().contains("anything"));
    assert_eq!(fixture.owners.read().unwrap().mapping.len(), 2);
    assert_eq!(fixture.caches.invalidated(), strings(&[""]));
}

#[tokio::test]
async fn cleanup_owned_mapping_values_lists_prefixes_and_name() {
    let fixture = test_fixture();
    let subject = server(
        r#"{"server_id":"srv-1","name":"My Server","transport":"http","alias":"Alias One","short_prefix":"abC"}"#,
    );
    let builder = fixture.builder();
    let mutator = fixture.mutator(&builder);
    let owned = mutator.owned_mapping_values(&subject);
    let computed = compute_short_server_prefix("srv-1", 0).unwrap();
    let expected: BTreeSet<String> = [
        String::from("Alias_One"),
        String::from("abC"),
        computed,
        String::from("srv-1"),
        String::from("My_Server"),
    ]
    .into_iter()
    .collect();
    assert_eq!(owned, expected);
}
