#[tokio::test]
async fn agent_servers_needs_an_agent() {
    let ceilings = over(FakeReads::standard());
    assert_eq!(
        ceilings.agent_servers(None, None).await.unwrap(),
        Vec::<String>::new()
    );
    assert_eq!(
        ceilings
            .agent_servers(Some(&user_auth()), None)
            .await
            .unwrap(),
        Vec::<String>::new()
    );
    assert!(!ceilings.reads.saw("agent:"));
}

#[tokio::test]
async fn agent_servers_unions_every_grant() {
    let ceilings = over(FakeReads {
        access_group_servers: Scripted::Value(names(&["srv-group"])),
        toolset: Scripted::Value(tool_grants(&[("srv-set", &["tool-a"])])),
        agent: Scripted::Value(Some(with_tools(
            with_groups(with_servers(row("perm-agent"), &["srv-a"]), &["g-1"]),
            "srv-tool",
            &["tool-a"],
        ))),
        ..FakeReads::standard()
    });
    assert_eq!(
        ceilings
            .agent_servers(Some(&agent_auth()), None)
            .await
            .unwrap(),
        names(&["srv-a", "srv-group", "srv-tool", "srv-set"])
    );
}

#[tokio::test]
async fn agent_servers_uses_the_prefetched_permission() {
    let prefetched = with_servers(row("perm-agent"), &["srv-a"]);
    let ceilings = over(FakeReads::standard());
    assert_eq!(
        ceilings
            .agent_servers(Some(&agent_auth()), Some(&prefetched))
            .await
            .unwrap(),
        names(&["srv-a"])
    );
    assert!(!ceilings.reads.saw("agent:"));
}

#[tokio::test]
async fn agent_servers_is_empty_without_a_permission() {
    let ceilings = over(FakeReads::standard());
    assert_eq!(
        ceilings
            .agent_servers(Some(&agent_auth()), None)
            .await
            .unwrap(),
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn agent_servers_returns_the_unloadable_getter_error() {
    let ceilings = over(FakeReads {
        agent: Scripted::Unloadable,
        ..FakeReads::standard()
    });
    let error = ceilings
        .agent_servers(Some(&agent_auth()), None)
        .await
        .unwrap_err();
    assert!(matches!(error, CeilingsError::Unloadable(_)));
}

#[tokio::test]
async fn agent_servers_reads_a_fault_by_agent_shape() {
    let plain = over(FakeReads {
        agent: Scripted::Value(Some(row("perm-agent"))),
        toolset: Scripted::Fault,
        ..FakeReads::standard()
    });
    assert_eq!(
        plain
            .agent_servers(Some(&agent_auth()), None)
            .await
            .unwrap(),
        Vec::<String>::new()
    );
    let managed = over(FakeReads {
        managed: true,
        agent: Scripted::Value(Some(row("perm-agent"))),
        toolset: Scripted::Fault,
        ..FakeReads::standard()
    });
    let error = managed
        .agent_servers(Some(&agent_auth()), None)
        .await
        .unwrap_err();
    assert!(matches!(error, CeilingsError::Failed(_)));
    let unloadable = over(FakeReads {
        agent: Scripted::Value(Some(row("perm-agent"))),
        toolset: Scripted::Unloadable,
        ..FakeReads::standard()
    });
    let error = unloadable
        .agent_servers(Some(&agent_auth()), None)
        .await
        .unwrap_err();
    assert!(matches!(error, CeilingsError::Unloadable(_)));
}

#[tokio::test]
async fn agent_access_group_ceiling_needs_an_agent_with_groups() {
    let ceilings = over(FakeReads::standard());
    assert_eq!(
        ceilings.agent_access_group_ceiling(None).await.unwrap(),
        None
    );
    assert_eq!(
        ceilings
            .agent_access_group_ceiling(Some(&user_auth()))
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        ceilings
            .agent_access_group_ceiling(Some(&agent_auth()))
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn agent_access_group_ceiling_sorts_and_dedupes_like_a_set() {
    let ceilings = over(FakeReads {
        agent_groups: Scripted::Value(Some(names(&["srv-b", "srv-a", "srv-b"]))),
        ..FakeReads::standard()
    });
    let ceiling = ceilings
        .agent_access_group_ceiling(Some(&agent_auth()))
        .await
        .unwrap();
    assert_eq!(ceiling, Some(names(&["srv-a", "srv-b"])));
}

#[tokio::test]
async fn agent_access_group_ceiling_keeps_an_empty_set() {
    let ceilings = over(FakeReads {
        agent_groups: Scripted::Value(Some(Vec::new())),
        ..FakeReads::standard()
    });
    assert_eq!(
        ceilings
            .agent_access_group_ceiling(Some(&agent_auth()))
            .await
            .unwrap(),
        Some(Vec::new())
    );
}

#[tokio::test]
async fn agent_access_group_ceiling_returns_the_read_error() {
    let ceilings = over(FakeReads {
        agent_groups: Scripted::Fault,
        ..FakeReads::standard()
    });
    let error = ceilings
        .agent_access_group_ceiling(Some(&agent_auth()))
        .await
        .unwrap_err();
    assert!(matches!(error, CeilingsError::Failed(_)));
}

#[tokio::test]
async fn agent_capped_servers_needs_a_cap() {
    let allowed = names(&["srv-a"]);
    assert_eq!(agent_capped_servers(&allowed, &[], None), None);
}

#[tokio::test]
async fn agent_capped_servers_filters_by_both_lists() {
    let allowed = names(&["srv-a", "srv-b", "srv-c"]);
    let direct = names(&["srv-a", "srv-b"]);
    assert_eq!(
        agent_capped_servers(&allowed, &direct, None),
        Some(names(&["srv-a", "srv-b"]))
    );
    let ceiling = names(&["srv-b", "srv-c"]);
    assert_eq!(
        agent_capped_servers(&allowed, &[], Some(&ceiling)),
        Some(names(&["srv-b", "srv-c"]))
    );
    assert_eq!(
        agent_capped_servers(&allowed, &direct, Some(&ceiling)),
        Some(names(&["srv-b"]))
    );
    let empty: Vec<String> = Vec::new();
    assert_eq!(
        agent_capped_servers(&allowed, &[], Some(&empty)),
        Some(Vec::new())
    );
    let repeated = names(&["srv-a", "srv-a"]);
    assert_eq!(
        agent_capped_servers(&repeated, &names(&["srv-a"]), None),
        Some(names(&["srv-a", "srv-a"]))
    );
}

#[tokio::test]
async fn agent_caller_ceiling_needs_an_echoed_caller() {
    let ceilings = over(FakeReads::standard());
    let allowed = names(&["srv-a"]);
    assert_eq!(
        ceilings
            .apply_agent_caller_ceiling(allowed.clone(), None)
            .await
            .unwrap(),
        capped(&["srv-a"], false)
    );
    assert_eq!(
        ceilings
            .apply_agent_caller_ceiling(allowed, Some(&agent_auth()))
            .await
            .unwrap(),
        capped(&["srv-a"], false)
    );
    assert!(!ceilings.reads.saw("team_servers:"));
}

#[tokio::test]
async fn agent_caller_ceiling_caps_by_a_non_empty_team() {
    let ceilings = over(FakeReads {
        caller: Scripted::Value(Some(user_auth())),
        team_servers: Scripted::Value(names(&["srv-a"])),
        ..FakeReads::standard()
    });
    let result = ceilings
        .apply_agent_caller_ceiling(names(&["srv-a", "srv-b"]), Some(&agent_auth()))
        .await
        .unwrap();
    assert_eq!(result, capped(&["srv-a"], true));
}

#[tokio::test]
async fn agent_caller_ceiling_keeps_the_list_for_an_empty_team() {
    let ceilings = over(FakeReads {
        caller: Scripted::Value(Some(user_auth())),
        ..FakeReads::standard()
    });
    let result = ceilings
        .apply_agent_caller_ceiling(names(&["srv-a", "srv-b"]), Some(&agent_auth()))
        .await
        .unwrap();
    assert_eq!(result, capped(&["srv-a", "srv-b"], false));
}

#[tokio::test]
async fn agent_caller_ceiling_then_applies_the_user_ceiling() {
    let ceilings = over(FakeReads {
        caller: Scripted::Value(Some(user_auth())),
        user: Scripted::Value(Some(with_servers(row("perm-user"), &["srv-a"]))),
        ..FakeReads::standard()
    });
    let result = ceilings
        .apply_agent_caller_ceiling(names(&["srv-a", "srv-b"]), Some(&agent_auth()))
        .await
        .unwrap();
    assert_eq!(result, capped(&["srv-a"], true));
    assert!(ceilings.reads.saw("user:user-1"));
}

#[tokio::test]
async fn agent_caller_ceiling_returns_the_team_error() {
    let ceilings = over(FakeReads {
        caller: Scripted::Value(Some(user_auth())),
        team_servers: Scripted::Unloadable,
        ..FakeReads::standard()
    });
    let error = ceilings
        .apply_agent_caller_ceiling(names(&["srv-a"]), Some(&agent_auth()))
        .await
        .unwrap_err();
    assert!(matches!(error, CeilingsError::Unloadable(_)));
}

#[tokio::test]
async fn agent_server_tools_needs_an_agent_with_a_permission() {
    let ceilings = over(FakeReads::standard());
    assert_eq!(
        ceilings
            .agent_tool_permissions_for_server("srv-a", None, None)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        ceilings
            .agent_tool_permissions_for_server("srv-a", Some(&user_auth()), None)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        ceilings
            .agent_tool_permissions_for_server("srv-a", Some(&agent_auth()), None)
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn agent_server_tools_reads_direct_grants_only_when_listed() {
    let ceilings = over(FakeReads {
        agent: Scripted::Value(Some(without_tools(row("perm-agent")))),
        toolset_server: Scripted::Value(Some(names(&["tool-set"]))),
        ..FakeReads::standard()
    });
    let tools = ceilings
        .agent_tool_permissions_for_server("srv-a", Some(&agent_auth()), None)
        .await
        .unwrap();
    assert_eq!(tools, Some(names(&["tool-set"])));
}

#[tokio::test]
async fn agent_server_tools_unions_direct_and_toolset_grants() {
    let ceilings = over(FakeReads {
        agent: Scripted::Value(Some(with_tools(row("perm-agent"), "srv-a", &["tool-a"]))),
        toolset_server: Scripted::Value(Some(names(&["tool-b"]))),
        ..FakeReads::standard()
    });
    let tools = ceilings
        .agent_tool_permissions_for_server("srv-a", Some(&agent_auth()), None)
        .await
        .unwrap();
    assert_eq!(tools, Some(names(&["tool-a", "tool-b"])));
}

#[tokio::test]
async fn agent_server_tools_ignores_a_wildcard() {
    let ceilings = over(FakeReads {
        agent: Scripted::Value(Some(with_tools(row("perm-agent"), "srv-a", &["*"]))),
        toolset_server: Scripted::Value(Some(names(&["tool-b"]))),
        ..FakeReads::standard()
    });
    let tools = ceilings
        .agent_tool_permissions_for_server("srv-a", Some(&agent_auth()), None)
        .await
        .unwrap();
    assert_eq!(tools, None);
}

#[tokio::test]
async fn agent_server_tools_reads_a_fault_by_agent_shape() {
    let plain = over(FakeReads {
        agent: Scripted::Value(Some(row("perm-agent"))),
        toolset_server: Scripted::Fault,
        ..FakeReads::standard()
    });
    assert_eq!(
        plain
            .agent_tool_permissions_for_server("srv-a", Some(&agent_auth()), None)
            .await
            .unwrap(),
        None
    );
    let managed = over(FakeReads {
        managed: true,
        agent: Scripted::Value(Some(row("perm-agent"))),
        toolset_server: Scripted::Fault,
        ..FakeReads::standard()
    });
    let error = managed
        .agent_tool_permissions_for_server("srv-a", Some(&agent_auth()), None)
        .await
        .unwrap_err();
    assert!(matches!(error, CeilingsError::Failed(_)));
    let unloadable = over(FakeReads {
        agent: Scripted::Value(Some(row("perm-agent"))),
        toolset_server: Scripted::Unloadable,
        ..FakeReads::standard()
    });
    let error = unloadable
        .agent_tool_permissions_for_server("srv-a", Some(&agent_auth()), None)
        .await
        .unwrap_err();
    assert!(matches!(error, CeilingsError::Unloadable(_)));
}

#[tokio::test]
async fn agent_server_tools_uses_the_prefetched_permission() {
    let prefetched = with_tools(row("perm-agent"), "srv-a", &["tool-a"]);
    let ceilings = over(FakeReads::standard());
    let tools = ceilings
        .agent_tool_permissions_for_server("srv-a", Some(&agent_auth()), Some(&prefetched))
        .await
        .unwrap();
    assert_eq!(tools, Some(names(&["tool-a"])));
    assert!(!ceilings.reads.saw("agent:"));
}
