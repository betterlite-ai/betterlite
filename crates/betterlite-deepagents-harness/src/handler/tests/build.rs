#[rstest]
#[case::ask_with_no_disabled_tools(PermissionMode::Ask, &[], true, true)]
#[case::read_only(PermissionMode::ReadOnly, &[], false, false)]
#[case::edit(PermissionMode::Edit, &[], true, false)]
#[case::full_with_write_disabled(PermissionMode::Full, &["write"], false, true)]
#[case::full_with_edit_disabled(PermissionMode::Full, &["edit"], false, true)]
#[case::full_with_delete_disabled(PermissionMode::Full, &["delete"], false, true)]
#[case::full_with_bash_disabled(PermissionMode::Full, &["bash"], true, false)]
#[case::full_with_read_tool_disabled(PermissionMode::Full, &["ls"], true, true)]
#[tokio::test]
async fn build_computes_backend_permissions(
    #[case] permissions: PermissionMode,
    #[case] disabled: &[&str],
    #[case] writable: bool,
    #[case] allow_execute: bool,
) {
    let (runtime, plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    ctx.settings.permissions = permissions;
    ctx.settings.disable_tools = disabled.iter().map(|name| (*name).to_owned()).collect();
    handler.start(&mut ctx).await.unwrap();
    let spec = built_spec(&plan);
    assert_eq!(
        spec.permissions,
        Permissions {
            writable,
            allow_execute
        }
    );
}

#[rstest]
#[case::nothing_blocked(PermissionMode::Ask, &[], false)]
#[case::read_only(PermissionMode::ReadOnly, &[], true)]
#[case::one_disabled_tool(PermissionMode::Full, &["ls"], true)]
#[tokio::test]
async fn build_passes_a_tool_filter_only_when_tools_are_blocked(
    #[case] permissions: PermissionMode,
    #[case] disabled: &[&str],
    #[case] has_filter: bool,
) {
    let (runtime, plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    ctx.settings.permissions = permissions;
    ctx.settings.disable_tools = disabled.iter().map(|name| (*name).to_owned()).collect();
    handler.start(&mut ctx).await.unwrap();
    let spec = built_spec(&plan);
    assert_eq!(spec.middleware.is_some(), has_filter);
}

#[tokio::test]
async fn build_the_tool_filter_lists_the_blocked_tools() {
    let (runtime, plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    ctx.settings.permissions = PermissionMode::ReadOnly;
    handler.start(&mut ctx).await.unwrap();
    let spec = built_spec(&plan);
    let filter = spec.middleware.unwrap();
    let blocked: Vec<&str> = filter.blocked.iter().map(String::as_str).collect();
    assert_eq!(blocked, ["delete", "edit_file", "execute", "write_file"]);
}

#[rstest]
#[case::ask(
    PermissionMode::Ask,
    &[],
    true,
    &["delete", "edit_file", "execute", "write_file"]
)]
#[case::ask_with_bash_disabled(
    PermissionMode::Ask,
    &["bash"],
    true,
    &["delete", "edit_file", "write_file"]
)]
#[case::full(PermissionMode::Full, &[], false, &[])]
#[tokio::test]
async fn build_interrupts_only_sessions_that_ask(
    #[case] permissions: PermissionMode,
    #[case] disabled: &[&str],
    #[case] has_policy: bool,
    #[case] expected: &[&str],
) {
    let (runtime, plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    ctx.settings.permissions = permissions;
    ctx.settings.disable_tools = disabled.iter().map(|name| (*name).to_owned()).collect();
    handler.start(&mut ctx).await.unwrap();
    let spec = built_spec(&plan);
    assert_eq!(spec.interrupt_on.is_some(), has_policy);
    let tools: Vec<&str> = spec.interrupt_on.as_ref().map_or_else(Vec::new, |entries| {
        entries.keys().map(String::as_str).collect()
    });
    assert_eq!(tools, expected);
}

#[tokio::test]
async fn build_carries_the_model_instructions_policy_and_schema() {
    let (runtime, plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    ctx.settings.instructions = Some("be helpful".to_owned());
    ctx.settings.permissions = PermissionMode::Ask;
    ctx.settings.output_schema = Some(json_object(r#"{"title":"Answer","type":"object"}"#));
    handler.start(&mut ctx).await.unwrap();
    let spec = built_spec(&plan);
    assert_eq!(spec.chat_model.model, "openai/gpt-4o");
    assert!(spec.chat_model.api_base.is_none());
    assert!(spec.chat_model.api_key.is_none());
    assert!(spec.chat_model.extra_headers.is_none());
    assert_eq!(spec.system_prompt.as_deref(), Some("be helpful"));
    assert_eq!(
        spec.output_schema,
        Some(json_object(r#"{"title":"Answer","type":"object"}"#))
    );
    assert!(spec.interrupt_on.is_some());
    assert_eq!(handler.skip_tools, BTreeSet::from(["Answer".to_owned()]));
}

#[tokio::test]
async fn build_defaults_the_thread_id_and_keeps_it_on_a_second_start() {
    let (runtime, plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.start(&mut ctx).await.unwrap();
    assert_eq!(handler.thread_id.as_deref(), Some("session-1"));
    handler.resume(&mut ctx, "native-7").await.unwrap();
    handler.start(&mut ctx).await.unwrap();
    assert_eq!(handler.thread_id.as_deref(), Some("native-7"));
    assert!(handler.graph.is_some());
    assert_eq!(plan.specs.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn build_resume_replaces_the_thread_id() {
    let (runtime, _plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.resume(&mut ctx, "native-7").await.unwrap();
    assert_eq!(handler.thread_id.as_deref(), Some("native-7"));
    assert_eq!(handler.native_session_id().as_deref(), Some("native-7"));
}

#[tokio::test]
async fn build_stop_twice_is_fine() {
    let (runtime, _plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.start(&mut ctx).await.unwrap();
    handler.stop(&mut ctx).await.unwrap();
    assert!(handler.graph.is_none());
    handler.stop(&mut ctx).await.unwrap();
}

#[tokio::test]
async fn build_fails_with_the_install_hint_when_the_runtime_is_missing() {
    let mut handler = DeepAgentsHandler {
        config: DeepAgentsHarnessConfig,
        runtime: MissingRuntime,
        graph: None,
        thread_id: None,
        skip_tools: BTreeSet::new(),
    };
    let mut ctx = context("/work");
    let error = handler.start(&mut ctx).await.unwrap_err();
    assert!(matches!(error, HarnessError::InstallFailed(_)));
    assert!(error.to_string().contains(INSTALL_HINT));
    assert!(handler.graph.is_none());
    assert!(handler.thread_id.is_none());
}

#[tokio::test]
async fn build_fails_with_the_install_hint_before_writing_skills() {
    let mut handler = DeepAgentsHandler {
        config: DeepAgentsHarnessConfig,
        runtime: MissingRuntime,
        graph: None,
        thread_id: None,
        skip_tools: BTreeSet::new(),
    };
    let mut ctx = context("/work");
    ctx.settings.skills = vec!["/no/such/skill".to_owned()];
    let error = handler.start(&mut ctx).await.unwrap_err();
    assert!(matches!(error, HarnessError::InstallFailed(_)));
    assert!(ctx.sandbox.writes.lock().unwrap().is_empty());
    assert!(handler.graph.is_none());
}

#[tokio::test]
async fn build_rejects_options_of_another_harness() {
    let (runtime, _plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    ctx.settings.options = Some(HarnessOptions::ClaudeCode(ClaudeCodeOptions::default()));
    let error = handler.start(&mut ctx).await.unwrap_err();
    assert!(matches!(error, HarnessError::OptionsMismatch { .. }));
    assert!(handler.graph.is_none());
}

#[tokio::test]
async fn build_adds_the_general_subagent_with_the_filter_when_tools_are_blocked() {
    let (runtime, plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    ctx.settings.permissions = PermissionMode::ReadOnly;
    handler.start(&mut ctx).await.unwrap();
    let spec = built_spec(&plan);
    assert_eq!(spec.subagents.len(), 1);
    let general = spec.subagents.first().unwrap();
    assert_eq!(
        general.get("name").and_then(|name| name.as_str()),
        Some(GENERAL_NAME)
    );
    let filter = general.get("middleware").unwrap().as_array().unwrap();
    assert_eq!(filter.len(), 1);
    let entry = filter.first().unwrap();
    assert_eq!(
        entry.get("name").and_then(|name| name.as_str()),
        Some(TOOL_FILTER_NAME)
    );
    let blocked: Vec<&str> = entry
        .get("blocked")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool.as_str().unwrap())
        .collect();
    assert_eq!(blocked, ["delete", "edit_file", "execute", "write_file"]);
}

#[tokio::test]
async fn build_keeps_the_general_subagent_out_when_nothing_is_blocked() {
    let (runtime, plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.start(&mut ctx).await.unwrap();
    let spec = built_spec(&plan);
    assert!(spec.subagents.is_empty());
    assert!(spec.middleware.is_none());
}

#[tokio::test]
async fn build_keeps_the_users_general_subagent_unchanged() {
    let (runtime, plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    ctx.settings.permissions = PermissionMode::ReadOnly;
    let user = json_object(r#"{"name": "general-purpose", "description": "mine"}"#);
    ctx.settings.options = Some(HarnessOptions::DeepAgents(DeepAgentsOptions {
        subagents: vec![user.clone()],
        recursion_limit: None,
    }));
    handler.start(&mut ctx).await.unwrap();
    let spec = built_spec(&plan);
    assert_eq!(spec.subagents, vec![user]);
}

#[tokio::test]
async fn build_puts_user_subagents_after_the_general_one() {
    let (runtime, plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    ctx.settings.disable_tools = vec!["bash".to_owned()];
    let user = json_object(r#"{"name": "researcher", "description": "finds things"}"#);
    ctx.settings.options = Some(HarnessOptions::DeepAgents(DeepAgentsOptions {
        subagents: vec![user.clone()],
        recursion_limit: None,
    }));
    handler.start(&mut ctx).await.unwrap();
    let spec = built_spec(&plan);
    assert_eq!(spec.subagents.len(), 2);
    assert_eq!(
        spec.subagents
            .first()
            .unwrap()
            .get("name")
            .and_then(|name| name.as_str()),
        Some(GENERAL_NAME)
    );
    assert_eq!(spec.subagents.get(1), Some(&user));
}

#[tokio::test]
async fn build_copies_skills_under_the_workdir_and_passes_the_virtual_dir() {
    let (runtime, plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    let skill = tempdir();
    let skill_dir = skill.path().join("my-skill");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(skill_dir.join("SKILL.md"), "# skill").unwrap();
    ctx.settings.skills = vec![skill_dir.to_str().unwrap().to_owned()];
    handler.start(&mut ctx).await.unwrap();
    let spec = built_spec(&plan);
    assert_eq!(spec.skills, Some(vec!["/.deepagents/skills/".to_owned()]));
    let writes = ctx.sandbox.writes.lock().unwrap();
    assert_eq!(writes.len(), 1);
    let (path, data) = writes.first_key_value().unwrap();
    assert_eq!(path, "/work/.deepagents/skills/my-skill/SKILL.md");
    assert_eq!(data.as_slice(), b"# skill");
}

#[tokio::test]
async fn build_without_skills_passes_none_and_copies_nothing() {
    let (runtime, plan) = FakeRuntime::new();
    let mut handler = handler(runtime);
    let mut ctx = context("/work");
    handler.start(&mut ctx).await.unwrap();
    let spec = built_spec(&plan);
    assert!(spec.skills.is_none());
    assert!(ctx.sandbox.writes.lock().unwrap().is_empty());
}
