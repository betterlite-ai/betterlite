#[tokio::test]
async fn tool_user_ceiling_skips_a_keyless_source() {
    let ceilings = over(FakeReads::standard());
    let tools = ceilings
        .apply_user_tool_ceiling(
            Some(names(&["tool-a"])),
            "srv-a",
            Some(&user_auth()),
            CeilingSource::Keyless,
        )
        .await
        .unwrap();
    assert_eq!(tools, Some(names(&["tool-a"])));
    assert!(ceilings.reads.calls().is_empty());
}

#[tokio::test]
async fn tool_user_ceiling_denies_on_a_read_fault() {
    let failed = over(FakeReads {
        user: Scripted::Fault,
        ..FakeReads::standard()
    });
    assert_eq!(
        failed
            .apply_user_tool_ceiling(
                Some(names(&["tool-a"])),
                "srv-a",
                Some(&user_auth()),
                CeilingSource::Key,
            )
            .await
            .unwrap(),
        Some(Vec::new())
    );
    let unloadable = over(FakeReads {
        user: Scripted::Unloadable,
        ..FakeReads::standard()
    });
    assert_eq!(
        unloadable
            .apply_user_tool_ceiling(
                Some(names(&["tool-a"])),
                "srv-a",
                Some(&user_auth()),
                CeilingSource::Key,
            )
            .await
            .unwrap(),
        Some(Vec::new())
    );
}

#[tokio::test]
async fn tool_user_ceiling_leaves_the_tools_without_grants() {
    let ceilings = over(FakeReads::standard());
    assert_eq!(
        ceilings
            .apply_user_tool_ceiling(
                Some(names(&["tool-a"])),
                "srv-a",
                Some(&user_auth()),
                CeilingSource::Key,
            )
            .await
            .unwrap(),
        Some(names(&["tool-a"]))
    );
    let empty = over(FakeReads {
        user: Scripted::Value(Some(row("perm-user"))),
        ..FakeReads::standard()
    });
    assert_eq!(
        empty
            .apply_user_tool_ceiling(None, "srv-a", Some(&user_auth()), CeilingSource::Key)
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn tool_user_ceiling_adopts_and_intersects_the_grants() {
    let ceilings = over(FakeReads {
        user: Scripted::Value(Some(with_tools(row("perm-user"), "srv-a", &["tool-a"]))),
        ..FakeReads::standard()
    });
    assert_eq!(
        ceilings
            .apply_user_tool_ceiling(None, "srv-a", Some(&user_auth()), CeilingSource::Key)
            .await
            .unwrap(),
        Some(names(&["tool-a"]))
    );
    assert_eq!(
        ceilings
            .apply_user_tool_ceiling(
                Some(names(&["tool-b", "tool-a"])),
                "srv-a",
                Some(&user_auth()),
                CeilingSource::Key,
            )
            .await
            .unwrap(),
        Some(names(&["tool-a"]))
    );
    assert!(ceilings.reads.saw("user:user-1"));
}

#[tokio::test]
async fn tool_user_ceiling_returns_a_toolset_error() {
    let ceilings = over(FakeReads {
        user: Scripted::Value(Some(row("perm-user"))),
        toolset_server: Scripted::Unloadable,
        ..FakeReads::standard()
    });
    let error = ceilings
        .apply_user_tool_ceiling(None, "srv-a", Some(&user_auth()), CeilingSource::Key)
        .await
        .unwrap_err();
    assert!(matches!(error, CeilingsError::Unloadable(_)));
}

#[tokio::test]
async fn tool_end_user_ceiling_needs_an_end_user_and_a_database() {
    let ceilings = over(FakeReads::standard());
    let allowed = Some(names(&["tool-a"]));
    assert_eq!(
        ceilings
            .apply_end_user_tool_ceiling(allowed.clone(), "srv-a", None)
            .await
            .unwrap(),
        allowed
    );
    assert_eq!(
        ceilings
            .apply_end_user_tool_ceiling(allowed, "srv-a", Some(&user_auth()))
            .await
            .unwrap(),
        Some(names(&["tool-a"]))
    );
    let offline = over(FakeReads {
        store: StoreState {
            database: false,
            fresh: false,
        },
        ..FakeReads::standard()
    });
    assert_eq!(
        offline
            .apply_end_user_tool_ceiling(Some(names(&["tool-a"])), "srv-a", Some(&end_user_auth()))
            .await
            .unwrap(),
        Some(names(&["tool-a"]))
    );
    assert!(!offline.reads.saw("end_user:"));
}

#[tokio::test]
async fn tool_end_user_ceiling_leaves_the_tools_without_a_permission() {
    let ceilings = over(FakeReads::standard());
    assert_eq!(
        ceilings
            .apply_end_user_tool_ceiling(Some(names(&["tool-a"])), "srv-a", Some(&end_user_auth()))
            .await
            .unwrap(),
        Some(names(&["tool-a"]))
    );
}

#[tokio::test]
async fn tool_end_user_ceiling_intersects_the_grants() {
    let ceilings = over(FakeReads {
        end_user: Scripted::Value(Some(with_tools(row("perm-eu"), "srv-a", &["tool-a"]))),
        ..FakeReads::standard()
    });
    assert_eq!(
        ceilings
            .apply_end_user_tool_ceiling(
                Some(names(&["tool-a", "tool-b"])),
                "srv-a",
                Some(&end_user_auth()),
            )
            .await
            .unwrap(),
        Some(names(&["tool-a"]))
    );
}

#[tokio::test]
async fn tool_end_user_ceiling_returns_the_read_error() {
    let ceilings = over(FakeReads {
        end_user: Scripted::Unloadable,
        ..FakeReads::standard()
    });
    let error = ceilings
        .apply_end_user_tool_ceiling(None, "srv-a", Some(&end_user_auth()))
        .await
        .unwrap_err();
    assert!(matches!(error, CeilingsError::Unloadable(_)));
}

#[tokio::test]
async fn tool_caller_ceiling_needs_an_echoed_caller() {
    let ceilings = over(FakeReads::standard());
    let allowed = Some(names(&["tool-a"]));
    assert_eq!(
        ceilings
            .apply_agent_caller_tool_ceiling(allowed.clone(), "srv-a", None)
            .await
            .unwrap(),
        allowed
    );
    assert_eq!(
        ceilings
            .apply_agent_caller_tool_ceiling(allowed, "srv-a", Some(&agent_auth()))
            .await
            .unwrap(),
        Some(names(&["tool-a"]))
    );
    assert!(!ceilings.reads.saw("team:"));
}

#[tokio::test]
async fn tool_caller_ceiling_denies_when_the_team_cannot_be_read() {
    let team_fault = over(FakeReads {
        caller: Scripted::Value(Some(user_auth())),
        team: Scripted::Fault,
        ..FakeReads::standard()
    });
    assert_eq!(
        team_fault
            .apply_agent_caller_tool_ceiling(
                Some(names(&["tool-a"])),
                "srv-a",
                Some(&agent_auth()),
            )
            .await
            .unwrap(),
        Some(Vec::new())
    );
    let toolset_fault = over(FakeReads {
        caller: Scripted::Value(Some(user_auth())),
        toolset_server: Scripted::Fault,
        ..FakeReads::standard()
    });
    assert_eq!(
        toolset_fault
            .apply_agent_caller_tool_ceiling(
                Some(names(&["tool-a"])),
                "srv-a",
                Some(&agent_auth()),
            )
            .await
            .unwrap(),
        Some(Vec::new())
    );
}

#[tokio::test]
async fn tool_caller_ceiling_intersects_the_team_grants() {
    let ceilings = over(FakeReads {
        caller: Scripted::Value(Some(user_auth())),
        team: Scripted::Value(Some(with_tools(row("perm-team"), "srv-a", &["tool-a"]))),
        ..FakeReads::standard()
    });
    assert_eq!(
        ceilings
            .apply_agent_caller_tool_ceiling(
                Some(names(&["tool-a", "tool-b"])),
                "srv-a",
                Some(&agent_auth()),
            )
            .await
            .unwrap(),
        Some(names(&["tool-a"]))
    );
}

#[tokio::test]
async fn tool_caller_ceiling_then_applies_the_user_ceiling() {
    let ceilings = over(FakeReads {
        caller: Scripted::Value(Some(user_auth())),
        user: Scripted::Value(Some(with_tools(row("perm-user"), "srv-a", &["tool-b"]))),
        ..FakeReads::standard()
    });
    assert_eq!(
        ceilings
            .apply_agent_caller_tool_ceiling(
                Some(names(&["tool-a"])),
                "srv-a",
                Some(&agent_auth()),
            )
            .await
            .unwrap(),
        Some(Vec::new())
    );
    assert!(ceilings.reads.saw("user:user-1"));
}

#[tokio::test]
async fn tool_agent_org_ceiling_needs_an_auth() {
    let ceilings = over(FakeReads::standard());
    let allowed = Some(names(&["tool-a"]));
    assert_eq!(
        ceilings
            .apply_agent_and_org_tool_ceilings(allowed.clone(), "srv-a", None, CeilingSource::Key)
            .await
            .unwrap(),
        allowed
    );
    assert_eq!(
        ceilings
            .apply_agent_and_org_tool_ceilings(
                allowed,
                "srv-a",
                Some(&user_auth()),
                CeilingSource::Key
            )
            .await
            .unwrap(),
        Some(names(&["tool-a"]))
    );
    assert!(!ceilings.reads.saw("agent:"));
}

#[tokio::test]
async fn tool_agent_org_ceiling_applies_the_agent_step() {
    let bounded = over(FakeReads {
        agent: Scripted::Value(Some(with_tools(row("perm-agent"), "srv-a", &["tool-a"]))),
        ..FakeReads::standard()
    });
    assert_eq!(
        bounded
            .apply_agent_and_org_tool_ceilings(
                Some(names(&["tool-a", "tool-b"])),
                "srv-a",
                Some(&agent_auth()),
                CeilingSource::Key,
            )
            .await
            .unwrap(),
        Some(names(&["tool-a"]))
    );
    let unrestricted = over(FakeReads {
        agent: Scripted::Value(Some(without_tools(row("perm-agent")))),
        ..FakeReads::standard()
    });
    assert_eq!(
        unrestricted
            .apply_agent_and_org_tool_ceilings(
                Some(names(&["tool-a"])),
                "srv-a",
                Some(&agent_auth()),
                CeilingSource::Key,
            )
            .await
            .unwrap(),
        Some(names(&["tool-a"]))
    );
}

#[tokio::test]
async fn tool_agent_org_ceiling_returns_the_agent_read_error() {
    let ceilings = over(FakeReads {
        agent: Scripted::Unloadable,
        ..FakeReads::standard()
    });
    let error = ceilings
        .apply_agent_and_org_tool_ceilings(None, "srv-a", Some(&agent_auth()), CeilingSource::Key)
        .await
        .unwrap_err();
    assert!(matches!(error, CeilingsError::Unloadable(_)));
}

#[tokio::test]
async fn tool_agent_org_ceiling_reads_an_org_fault_by_source_kind() {
    let key = over(FakeReads {
        org: Scripted::Fault,
        ..FakeReads::standard()
    })
    .apply_agent_and_org_tool_ceilings(
        Some(names(&["tool-a"])),
        "srv-a",
        Some(&org_auth()),
        CeilingSource::Key,
    )
    .await
    .unwrap();
    assert_eq!(key, Some(names(&["tool-a"])));
    let keyless = over(FakeReads {
        org: Scripted::Fault,
        ..FakeReads::standard()
    });
    let error = keyless
        .apply_agent_and_org_tool_ceilings(
            Some(names(&["tool-a"])),
            "srv-a",
            Some(&org_auth()),
            CeilingSource::Keyless,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, CeilingsError::Failed(_)));
    let unloadable = over(FakeReads {
        org: Scripted::Unloadable,
        ..FakeReads::standard()
    });
    let error = unloadable
        .apply_agent_and_org_tool_ceilings(
            Some(names(&["tool-a"])),
            "srv-a",
            Some(&org_auth()),
            CeilingSource::Key,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, CeilingsError::Unloadable(_)));
}

#[tokio::test]
async fn tool_agent_org_ceiling_intersects_the_org_grants() {
    let ceilings = over(FakeReads {
        org: Scripted::Value(Some(with_tools(row("perm-org"), "srv-a", &["tool-a"]))),
        ..FakeReads::standard()
    });
    assert_eq!(
        ceilings
            .apply_agent_and_org_tool_ceilings(
                Some(names(&["tool-a", "tool-b"])),
                "srv-a",
                Some(&org_auth()),
                CeilingSource::Key,
            )
            .await
            .unwrap(),
        Some(names(&["tool-a"]))
    );
}

#[tokio::test]
async fn tool_agent_org_ceiling_ignores_an_empty_org_tool_map() {
    let ceilings = over(FakeReads {
        org: Scripted::Value(Some(without_tools(row("perm-org")))),
        toolset_server: Scripted::Value(Some(names(&["tool-a"]))),
        ..FakeReads::standard()
    });
    assert_eq!(
        ceilings
            .apply_agent_and_org_tool_ceilings(None, "srv-a", Some(&org_auth()), CeilingSource::Key)
            .await
            .unwrap(),
        Some(names(&["tool-a"]))
    );
}

#[tokio::test]
async fn tool_agent_org_ceiling_returns_an_org_toolset_error() {
    let ceilings = over(FakeReads {
        org: Scripted::Value(Some(row("perm-org"))),
        toolset_server: Scripted::Fault,
        ..FakeReads::standard()
    });
    let error = ceilings
        .apply_agent_and_org_tool_ceilings(None, "srv-a", Some(&org_auth()), CeilingSource::Key)
        .await
        .unwrap_err();
    assert!(matches!(error, CeilingsError::Failed(_)));
}

#[tokio::test]
async fn tool_user_ceiling_dedupes_the_intersection_like_a_set() {
    let ceilings = over(FakeReads {
        user: Scripted::Value(Some(with_tools(row("perm-user"), "srv-a", &["tool-a"]))),
        ..FakeReads::standard()
    });
    assert_eq!(
        ceilings
            .apply_user_tool_ceiling(
                Some(names(&["tool-a", "tool-a", "tool-b"])),
                "srv-a",
                Some(&user_auth()),
                CeilingSource::Key,
            )
            .await
            .unwrap(),
        Some(names(&["tool-a"]))
    );
}

#[tokio::test]
async fn tool_user_ceiling_ignores_a_wildcard_grant() {
    let ceilings = over(FakeReads {
        user: Scripted::Value(Some(with_tools(row("perm-user"), "srv-a", &["*"]))),
        ..FakeReads::standard()
    });
    assert_eq!(
        ceilings
            .apply_user_tool_ceiling(
                Some(names(&["tool-a"])),
                "srv-a",
                Some(&user_auth()),
                CeilingSource::Key,
            )
            .await
            .unwrap(),
        Some(names(&["tool-a"]))
    );
}

#[tokio::test]
async fn tool_end_user_ceiling_adopts_the_grants() {
    let ceilings = over(FakeReads {
        end_user: Scripted::Value(Some(with_tools(row("perm-eu"), "srv-a", &["tool-a"]))),
        ..FakeReads::standard()
    });
    assert_eq!(
        ceilings
            .apply_end_user_tool_ceiling(None, "srv-a", Some(&end_user_auth()))
            .await
            .unwrap(),
        Some(names(&["tool-a"]))
    );
}

#[tokio::test]
async fn tool_end_user_ceiling_ignores_a_wildcard_grant() {
    let ceilings = over(FakeReads {
        end_user: Scripted::Value(Some(with_tools(row("perm-eu"), "srv-a", &["*"]))),
        ..FakeReads::standard()
    });
    assert_eq!(
        ceilings
            .apply_end_user_tool_ceiling(Some(names(&["tool-a"])), "srv-a", Some(&end_user_auth()))
            .await
            .unwrap(),
        Some(names(&["tool-a"]))
    );
}

#[tokio::test]
async fn tool_agent_org_ceiling_intersects_the_agent_and_org_grants() {
    let ceilings = over(FakeReads {
        agent: Scripted::Value(Some(with_tools(
            row("perm-agent"),
            "srv-a",
            &["tool-a", "tool-b"],
        ))),
        org: Scripted::Value(Some(with_tools(
            row("perm-org"),
            "srv-a",
            &["tool-b", "tool-c"],
        ))),
        ..FakeReads::standard()
    });
    assert_eq!(
        ceilings
            .apply_agent_and_org_tool_ceilings(
                None,
                "srv-a",
                Some(&agent_auth()),
                CeilingSource::Key
            )
            .await
            .unwrap(),
        Some(names(&["tool-b"]))
    );
}
