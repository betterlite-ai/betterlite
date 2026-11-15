#[test]
fn handler_init_assigns_the_stable_id_and_records_the_source() {
    let mut handler = handler();
    let initialized = handler
        .initialize_guardrail(
            guardrail(None, "pii-guard", BEDROCK_PARAMS),
            GuardrailDefinitionLocation::Config,
        )
        .unwrap();
    assert_eq!(initialized.guardrail_id.as_deref(), Some(PII_GUARD_ID));
    assert_eq!(initialized.team_id, None);
    assert_eq!(
        handler.get_source(PII_GUARD_ID),
        Some(GuardrailDefinitionLocation::Config)
    );
    assert_eq!(handler.list_in_memory_guardrails().len(), 1);
    assert_eq!(handler.all_callbacks().count(), 1);
}

#[test]
fn handler_init_uses_the_next_seed_for_a_second_guardrail_of_the_same_name() {
    let mut handler = handler();
    let first = handler
        .initialize_guardrail(
            guardrail(None, "pii-guard", BEDROCK_PARAMS),
            GuardrailDefinitionLocation::Config,
        )
        .unwrap();
    let second = handler
        .initialize_guardrail(
            guardrail(None, "pii-guard", BEDROCK_PARAMS),
            GuardrailDefinitionLocation::Config,
        )
        .unwrap();
    assert_eq!(first.guardrail_id.as_deref(), Some(PII_GUARD_ID));
    assert_eq!(second.guardrail_id.as_deref(), Some(PII_GUARD_ID_1));
    assert_eq!(handler.list_in_memory_guardrails().len(), 2);
}

#[test]
fn handler_init_treats_an_empty_id_as_absent() {
    let mut handler = handler();
    let initialized = handler
        .initialize_guardrail(
            guardrail(Some(""), "pii-guard", BEDROCK_PARAMS),
            GuardrailDefinitionLocation::Config,
        )
        .unwrap();
    assert_eq!(initialized.guardrail_id.as_deref(), Some(PII_GUARD_ID));
}

#[test]
fn handler_init_returns_the_held_guardrail_without_building() {
    let mut handler = handler();
    handler
        .initialize_guardrail(
            guardrail(Some("held"), "held-guard", BEDROCK_PARAMS),
            GuardrailDefinitionLocation::Config,
        )
        .unwrap();
    let again = handler
        .initialize_guardrail(
            guardrail(
                Some("held"),
                "held-guard",
                r#"{"guardrail":"grayswan","mode":"pre_call"}"#,
            ),
            GuardrailDefinitionLocation::Db,
        )
        .unwrap();
    assert_eq!(again.litellm_params.guardrail, "bedrock");
    assert_eq!(
        handler.get_source("held"),
        Some(GuardrailDefinitionLocation::Db)
    );
    assert_eq!(handler.all_callbacks().count(), 1);
}

#[test]
fn handler_init_replaces_environment_references() {
    let mut env = BTreeMap::new();
    env.insert("GUARD_KEY".to_owned(), "sk-live".to_owned());
    env.insert("GUARD_BASE".to_owned(), "https://guard.example".to_owned());
    let mut handler = handler_with(env, true, false);
    let initialized = handler
        .initialize_guardrail(
            guardrail(
                None,
                "lakera-guard",
                r#"{"guardrail":"lakera","mode":"pre_call","api_key":"os.environ/GUARD_KEY","api_base":"os.environ/GUARD_BASE"}"#,
            ),
            GuardrailDefinitionLocation::Config,
        )
        .unwrap();
    let fields = &initialized.litellm_params.base.fields;
    assert_eq!(
        fields.api_key.as_ref().map(|key| key.expose_secret()),
        Some("sk-live")
    );
    assert_eq!(fields.api_base.as_deref(), Some("https://guard.example"));
}

#[test]
fn handler_init_uses_none_text_for_a_missing_environment_variable() {
    let mut handler = handler();
    let initialized = handler
        .initialize_guardrail(
            guardrail(
                None,
                "lakera-guard",
                r#"{"guardrail":"lakera","mode":"pre_call","api_key":"os.environ/MISSING_KEY"}"#,
            ),
            GuardrailDefinitionLocation::Config,
        )
        .unwrap();
    assert_eq!(
        initialized
            .litellm_params
            .base
            .fields
            .api_key
            .as_ref()
            .map(|key| key.expose_secret()),
        Some("None")
    );
}

#[test]
fn handler_init_keeps_presidio_callbacks_in_order() {
    let mut handler = handler();
    let initialized = handler
        .initialize_guardrail(
            guardrail(None, "pii-guard", PRESIDIO_PARAMS),
            GuardrailDefinitionLocation::Config,
        )
        .unwrap();
    assert_eq!(initialized.guardrail_name, "pii-guard");
    let callbacks: Vec<&GuardrailCallback> = handler.all_callbacks().collect();
    assert_eq!(callbacks.len(), 3);
    assert!(matches!(
        &callbacks[0].spec,
        CallbackSpec::Presidio(args) if args.role == PresidioRole::Scan && !args.apply_to_output
    ));
    assert!(matches!(
        &callbacks[1].spec,
        CallbackSpec::Presidio(args) if args.role == PresidioRole::Restore
    ));
    assert!(matches!(
        &callbacks[2].spec,
        CallbackSpec::Presidio(args) if args.role == PresidioRole::Scan && args.apply_to_output
    ));
}

#[test]
fn handler_init_rejects_an_empty_guardrail_type() {
    let mut handler = handler();
    let error = handler
        .initialize_guardrail(
            guardrail(None, "empty-guard", r#"{"guardrail":"","mode":"pre_call"}"#),
            GuardrailDefinitionLocation::Config,
        )
        .unwrap_err();
    assert_eq!(error.to_string(), "guardrail_type is required");
    assert_eq!(handler.list_in_memory_guardrails().len(), 0);
}

#[test]
fn handler_init_rejects_an_unknown_kind_without_storing_it() {
    let mut handler = handler();
    let error = handler
        .initialize_guardrail(
            guardrail(
                None,
                "grey-guard",
                r#"{"guardrail":"grayswan","mode":"pre_call"}"#,
            ),
            GuardrailDefinitionLocation::Config,
        )
        .unwrap_err();
    assert_eq!(error.to_string(), "Unsupported guardrail: grayswan");
    assert_eq!(handler.list_in_memory_guardrails().len(), 0);
    assert_eq!(handler.all_callbacks().count(), 0);
}

#[test]
fn handler_init_passes_strict_modes_to_the_builders() {
    let mut handler = handler();
    let error = handler
        .initialize_guardrail(
            guardrail(
                None,
                "lakera-guard",
                r#"{"guardrail":"lakera","mode":"post_call"}"#,
            ),
            GuardrailDefinitionLocation::Config,
        )
        .unwrap_err();
    assert!(matches!(error, RegistryError::Base(_)));
}

#[test]
fn handler_init_applies_the_global_skip_tool_setting() {
    let mut handler = handler_with(BTreeMap::new(), true, true);
    let error = handler
        .initialize_guardrail(
            guardrail(
                None,
                "lakera-guard",
                r#"{"guardrail":"lakera","mode":"pre_call","scan_only_tool_results":true}"#,
            ),
            GuardrailDefinitionLocation::Config,
        )
        .unwrap_err();
    assert!(matches!(
        error,
        RegistryError::ScanOnlyWithSkipTool { guardrail_name } if guardrail_name == "lakera-guard"
    ));
}

#[test]
fn handler_init_marks_a_compression_callback() {
    let mut callbacks = factories()
        .build("bedrock", &params(BEDROCK_PARAMS), "compress", true)
        .unwrap();
    super::handler::mark_compression(&mut callbacks, "compresr");
    assert!(callbacks[0].is_compression());
    assert!(matches!(
        &callbacks[0].spec,
        CallbackSpec::Compression(CompressionKind::Compresr)
    ));
}

#[test]
fn handler_init_leaves_compression_providers_unsupported_for_now() {
    let mut handler = handler();
    let error = handler
        .initialize_guardrail(
            guardrail(
                None,
                "compress-guard",
                r#"{"guardrail":"headroom","mode":"pre_call"}"#,
            ),
            GuardrailDefinitionLocation::Config,
        )
        .unwrap_err();
    assert_eq!(error.to_string(), "Unsupported guardrail: headroom");
}

#[test]
fn handler_update_keeps_callbacks_when_params_are_unchanged() {
    let mut handler = handler();
    let initialized = handler
        .initialize_guardrail(
            guardrail(None, "pii-guard", BEDROCK_PARAMS),
            GuardrailDefinitionLocation::Config,
        )
        .unwrap();
    let id = initialized.guardrail_id.clone().unwrap();
    let mut changed_info = guardrail(Some(id.as_str()), "pii-guard", BEDROCK_PARAMS);
    changed_info.guardrail_info = Some(OpaqueParams(BTreeMap::from([(
        "team".to_owned(),
        GuardrailValue::Text("x".to_owned()),
    )])));
    handler
        .initialize_guardrail(
            guardrail(None, "later-guard", BEDROCK_PARAMS),
            GuardrailDefinitionLocation::Config,
        )
        .unwrap();
    handler
        .update_in_memory_guardrail(&id, changed_info, GuardrailDefinitionLocation::Db)
        .unwrap();
    // a rebuild would move the guardrail's callback behind the later one
    let names: Vec<Option<&str>> = handler
        .all_callbacks()
        .map(|callback| callback.base.settings.guardrail_name.as_deref())
        .collect();
    assert_eq!(names, [Some("pii-guard"), Some("later-guard")]);
    assert_eq!(
        handler.get_source(&id),
        Some(GuardrailDefinitionLocation::Db)
    );
    assert!(
        handler
            .get_guardrail_by_id(&id)
            .unwrap()
            .guardrail_info
            .is_some()
    );
}

#[test]
fn handler_update_rebuilds_when_the_params_change() {
    let mut handler = handler();
    let initialized = handler
        .initialize_guardrail(
            guardrail(None, "pii-guard", BEDROCK_PARAMS),
            GuardrailDefinitionLocation::Config,
        )
        .unwrap();
    let id = initialized.guardrail_id.clone().unwrap();
    handler
        .update_in_memory_guardrail(
            &id,
            guardrail(
                Some(id.as_str()),
                "pii-guard",
                r#"{"guardrail":"bedrock","mode":"post_call"}"#,
            ),
            GuardrailDefinitionLocation::Db,
        )
        .unwrap();
    let held = handler.get_guardrail_by_id(&id).unwrap();
    assert_eq!(
        held.litellm_params.mode,
        LitellmParamsMode::One("post_call".to_owned())
    );
    assert_eq!(
        handler
            .all_callbacks()
            .next()
            .unwrap()
            .base
            .settings
            .event_hook,
        Some(LitellmParamsMode::One("post_call".to_owned()))
    );
    assert_eq!(handler.all_callbacks().count(), 1);
}

#[test]
fn handler_update_rebuilds_when_the_name_changes() {
    let mut handler = handler();
    let initialized = handler
        .initialize_guardrail(
            guardrail(None, "pii-guard", BEDROCK_PARAMS),
            GuardrailDefinitionLocation::Config,
        )
        .unwrap();
    let id = initialized.guardrail_id.clone().unwrap();
    handler
        .update_in_memory_guardrail(
            &id,
            guardrail(Some(id.as_str()), "pii-renamed", BEDROCK_PARAMS),
            GuardrailDefinitionLocation::Db,
        )
        .unwrap();
    let held = handler.get_guardrail_by_id(&id).unwrap();
    assert_eq!(held.guardrail_name, "pii-renamed");
    assert_eq!(
        handler
            .all_callbacks()
            .next()
            .unwrap()
            .base
            .settings
            .guardrail_name
            .as_deref(),
        Some("pii-renamed")
    );
    assert_eq!(handler.list_in_memory_guardrails().len(), 1);
}

#[test]
fn handler_update_restores_the_previous_guardrail_when_the_new_one_fails() {
    let mut handler = handler();
    let initialized = handler
        .initialize_guardrail(
            guardrail(None, "pii-guard", BEDROCK_PARAMS),
            GuardrailDefinitionLocation::Config,
        )
        .unwrap();
    let id = initialized.guardrail_id.clone().unwrap();
    let error = handler
        .update_in_memory_guardrail(
            &id,
            guardrail(
                Some(id.as_str()),
                "pii-guard",
                r#"{"guardrail":"bedrock","mode":"logging_only"}"#,
            ),
            GuardrailDefinitionLocation::Db,
        )
        .unwrap_err();
    assert!(matches!(error, RegistryError::InitFailed { .. }));
    let message = error.to_string();
    assert!(message.starts_with("Guardrail initialization failed: "));
    assert!(message.contains("logging_only"));
    let held = handler.get_guardrail_by_id(&id).unwrap();
    assert_eq!(
        held.litellm_params.mode,
        LitellmParamsMode::One("pre_call".to_owned())
    );
    assert_eq!(handler.list_in_memory_guardrails().len(), 1);
    assert_eq!(handler.all_callbacks().count(), 1);
    assert_eq!(
        handler.get_source(&id),
        Some(GuardrailDefinitionLocation::Config)
    );
}

#[test]
fn handler_update_reinitialize_without_an_id_returns_none() {
    let mut handler = handler();
    let result = handler
        .reinitialize_guardrail(
            &guardrail(None, "pii-guard", BEDROCK_PARAMS),
            GuardrailDefinitionLocation::Config,
        )
        .unwrap();
    assert!(result.is_none());
    assert_eq!(handler.list_in_memory_guardrails().len(), 0);
}

#[test]
fn handler_update_reinitialize_with_an_empty_id_returns_none() {
    let mut handler = handler();
    let result = handler
        .reinitialize_guardrail(
            &guardrail(Some(""), "pii-guard", BEDROCK_PARAMS),
            GuardrailDefinitionLocation::Config,
        )
        .unwrap();
    assert!(result.is_none());
    assert_eq!(handler.list_in_memory_guardrails().len(), 0);
}

#[test]
fn handler_update_reinitialize_returns_the_new_guardrail() {
    let mut handler = handler();
    let initialized = handler
        .initialize_guardrail(
            guardrail(None, "pii-guard", BEDROCK_PARAMS),
            GuardrailDefinitionLocation::Config,
        )
        .unwrap();
    let id = initialized.guardrail_id.clone().unwrap();
    let result = handler
        .reinitialize_guardrail(
            &guardrail(
                Some(id.as_str()),
                "pii-guard",
                r#"{"guardrail":"bedrock","mode":"post_call"}"#,
            ),
            GuardrailDefinitionLocation::Db,
        )
        .unwrap()
        .unwrap();
    assert_eq!(result.guardrail_id.as_deref(), Some(id.as_str()));
    assert_eq!(
        handler.get_source(&id),
        Some(GuardrailDefinitionLocation::Db)
    );
}

#[test]
fn handler_update_delete_drops_guardrail_source_and_siblings() {
    let mut handler = handler();
    let initialized = handler
        .initialize_guardrail(
            guardrail(None, "pii-guard", PRESIDIO_PARAMS),
            GuardrailDefinitionLocation::Config,
        )
        .unwrap();
    let id = initialized.guardrail_id.clone().unwrap();
    assert_eq!(handler.all_callbacks().count(), 3);
    handler.delete_in_memory_guardrail(&id);
    assert!(handler.get_guardrail_by_id(&id).is_none());
    assert!(handler.get_source(&id).is_none());
    assert_eq!(handler.all_callbacks().count(), 0);
    assert_eq!(handler.list_in_memory_guardrails().len(), 0);
}

#[test]
fn handler_update_config_readers_hide_db_entries() {
    let mut handler = handler();
    let configured = handler
        .initialize_guardrail(
            guardrail(None, "config-guard", BEDROCK_PARAMS),
            GuardrailDefinitionLocation::Config,
        )
        .unwrap();
    let stored = handler
        .initialize_guardrail(
            guardrail(None, "db-guard", BEDROCK_PARAMS),
            GuardrailDefinitionLocation::Db,
        )
        .unwrap();
    let config_id = configured.guardrail_id.clone().unwrap();
    let db_id = stored.guardrail_id.clone().unwrap();
    let listed: Vec<String> = handler
        .list_config_guardrails()
        .iter()
        .map(|guardrail| guardrail.guardrail_name.clone())
        .collect();
    assert_eq!(listed, ["config-guard"]);
    assert!(handler.get_config_guardrail_by_id(&config_id).is_some());
    assert!(handler.get_config_guardrail_by_id(&db_id).is_none());
}
