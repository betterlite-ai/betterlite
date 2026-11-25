#[tokio::test]
async fn org_servers_unions_every_grant() {
    let ceilings = over(FakeReads {
        aliases: BTreeMap::from([(String::from("alias"), names(&["srv-b"]))]),
        access_group_servers: Scripted::Value(names(&["srv-group"])),
        toolset: Scripted::Value(tool_grants(&[("srv-set", &["tool-a"])])),
        org: Scripted::Value(Some(with_tools(
            with_groups(with_servers(row("perm-org"), &["srv-a", "alias"]), &["g-1"]),
            "srv-tool",
            &["tool-a"],
        ))),
        ..FakeReads::standard()
    });
    assert_eq!(
        ceilings.org_servers(Some(&org_auth())).await.unwrap(),
        CeilingServers::Servers(names(&[
            "srv-a",
            "srv-b",
            "srv-group",
            "srv-tool",
            "srv-set"
        ]))
    );
    assert!(ceilings.reads.saw("access_groups:[\"g-1\"]:false"));
    assert!(ceilings.reads.saw("toolsets:false"));
}

#[tokio::test]
async fn org_servers_answers_no_restriction_without_a_permission() {
    let ceilings = over(FakeReads::standard());
    assert_eq!(
        ceilings.org_servers(Some(&org_auth())).await.unwrap(),
        CeilingServers::NoRestriction
    );
}

#[tokio::test]
async fn org_servers_keeps_an_empty_union_as_servers() {
    let ceilings = over(FakeReads {
        org: Scripted::Value(Some(row("perm-org"))),
        ..FakeReads::standard()
    });
    assert_eq!(
        ceilings.org_servers(Some(&org_auth())).await.unwrap(),
        CeilingServers::Servers(Vec::new())
    );
}

#[tokio::test]
async fn org_servers_turns_a_failed_read_into_unresolved() {
    let failing = over(FakeReads {
        org: Scripted::Fault,
        ..FakeReads::standard()
    });
    assert_eq!(
        failing.org_servers(Some(&org_auth())).await.unwrap(),
        CeilingServers::Unresolved
    );
    let toolsets = over(FakeReads {
        org: Scripted::Value(Some(row("perm-org"))),
        toolset: Scripted::Fault,
        ..FakeReads::standard()
    });
    assert_eq!(
        toolsets.org_servers(Some(&org_auth())).await.unwrap(),
        CeilingServers::Unresolved
    );
}

#[tokio::test]
async fn org_servers_returns_the_unloadable_errors() {
    let named = over(FakeReads {
        org: Scripted::Unloadable,
        ..FakeReads::standard()
    });
    let error = named.org_servers(Some(&org_auth())).await.unwrap_err();
    assert!(matches!(error, CeilingsError::Unloadable(_)));
    let toolsets = over(FakeReads {
        org: Scripted::Value(Some(row("perm-org"))),
        toolset: Scripted::Unloadable,
        ..FakeReads::standard()
    });
    let refused = toolsets.org_servers(Some(&org_auth())).await.unwrap_err();
    assert!(matches!(refused, CeilingsError::Unloadable(_)));
}

#[tokio::test]
async fn org_ceiling_leaves_the_list_without_an_org() {
    let ceilings = over(FakeReads::standard());
    let allowed = names(&["srv-a"]);
    assert_eq!(
        ceilings
            .apply_primary_org_ceiling(allowed.clone(), None, false, CeilingSource::Key)
            .await
            .unwrap(),
        capped(&["srv-a"], false)
    );
    assert_eq!(
        ceilings
            .apply_primary_org_ceiling(allowed, Some(&user_auth()), false, CeilingSource::Key)
            .await
            .unwrap(),
        capped(&["srv-a"], false)
    );
    assert!(!ceilings.reads.saw("org:"));
}

#[tokio::test]
async fn org_ceiling_leaves_the_list_for_no_org_restriction() {
    let ceilings = over(FakeReads::standard());
    let allowed = names(&["srv-a"]);
    let unchanged = capped(&["srv-a"], false);
    assert_eq!(
        ceilings
            .apply_primary_org_ceiling(allowed.clone(), Some(&org_auth()), true, CeilingSource::Key)
            .await
            .unwrap(),
        unchanged
    );
    let empty = over(FakeReads {
        org: Scripted::Value(Some(row("perm-org"))),
        ..FakeReads::standard()
    });
    assert_eq!(
        empty
            .apply_primary_org_ceiling(allowed, Some(&org_auth()), true, CeilingSource::Key)
            .await
            .unwrap(),
        unchanged
    );
}

#[tokio::test]
async fn org_ceiling_reads_an_unresolved_org_by_source_kind() {
    let key = over(FakeReads {
        org: Scripted::Fault,
        ..FakeReads::standard()
    })
    .apply_primary_org_ceiling(
        names(&["srv-a"]),
        Some(&org_auth()),
        false,
        CeilingSource::Key,
    )
    .await
    .unwrap();
    assert_eq!(key, capped(&["srv-a"], false));
    let keyless = over(FakeReads {
        org: Scripted::Fault,
        ..FakeReads::standard()
    })
    .apply_primary_org_ceiling(
        names(&["srv-a"]),
        Some(&org_auth()),
        true,
        CeilingSource::Keyless,
    )
    .await
    .unwrap();
    assert_eq!(keyless, capped(&[], false));
}

#[tokio::test]
async fn org_ceiling_caps_a_lower_restriction_and_a_keyless_source() {
    let ceilings = over(FakeReads {
        org: Scripted::Value(Some(with_servers(row("perm-org"), &["srv-a"]))),
        ..FakeReads::standard()
    });
    let lowered = ceilings
        .apply_primary_org_ceiling(
            names(&["srv-a", "srv-b"]),
            Some(&org_auth()),
            true,
            CeilingSource::Key,
        )
        .await
        .unwrap();
    assert_eq!(lowered, capped(&["srv-a"], true));
    let keyless = ceilings
        .apply_primary_org_ceiling(
            names(&["srv-a", "srv-b"]),
            Some(&org_auth()),
            false,
            CeilingSource::Keyless,
        )
        .await
        .unwrap();
    assert_eq!(keyless, capped(&["srv-a"], true));
}

#[tokio::test]
async fn org_ceiling_caps_by_membership_rather_than_replacing() {
    let ceilings = over(FakeReads {
        org: Scripted::Value(Some(with_servers(row("perm-org"), &["srv-a"]))),
        ..FakeReads::standard()
    });
    let lowered = ceilings
        .apply_primary_org_ceiling(
            names(&["srv-b"]),
            Some(&org_auth()),
            true,
            CeilingSource::Key,
        )
        .await
        .unwrap();
    assert_eq!(lowered, capped(&[], true));
    let keyless = ceilings
        .apply_primary_org_ceiling(
            names(&["srv-b"]),
            Some(&org_auth()),
            false,
            CeilingSource::Keyless,
        )
        .await
        .unwrap();
    assert_eq!(keyless, capped(&[], true));
}

#[tokio::test]
async fn org_ceiling_replaces_a_key_list_without_a_lower_restriction() {
    let ceilings = over(FakeReads {
        org: Scripted::Value(Some(with_servers(row("perm-org"), &["srv-a"]))),
        ..FakeReads::standard()
    });
    let result = ceilings
        .apply_primary_org_ceiling(
            names(&["srv-b"]),
            Some(&org_auth()),
            false,
            CeilingSource::Key,
        )
        .await
        .unwrap();
    assert_eq!(result, capped(&["srv-a"], true));
}

#[tokio::test]
async fn org_ceiling_returns_an_unloadable_org() {
    let ceilings = over(FakeReads {
        org: Scripted::Unloadable,
        ..FakeReads::standard()
    });
    let error = ceilings
        .apply_primary_org_ceiling(
            names(&["srv-a"]),
            Some(&org_auth()),
            false,
            CeilingSource::Keyless,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, CeilingsError::Unloadable(_)));
}

#[tokio::test]
async fn end_user_servers_needs_an_end_user_and_a_database() {
    let ceilings = over(FakeReads {
        end_user: Scripted::Value(Some(with_servers(row("perm-eu"), &["srv-eu"]))),
        ..FakeReads::standard()
    });
    assert_eq!(
        ceilings.end_user_servers(None).await.unwrap(),
        Vec::<String>::new()
    );
    assert_eq!(
        ceilings.end_user_servers(Some(&user_auth())).await.unwrap(),
        Vec::<String>::new()
    );
    let offline = over(FakeReads {
        store: StoreState {
            database: false,
            fresh: false,
        },
        end_user: Scripted::Value(Some(with_servers(row("perm-eu"), &["srv-eu"]))),
        ..FakeReads::standard()
    });
    assert_eq!(
        offline
            .end_user_servers(Some(&end_user_auth()))
            .await
            .unwrap(),
        Vec::<String>::new()
    );
    assert!(!offline.reads.saw("end_user:"));
}

#[tokio::test]
async fn end_user_servers_unions_without_toolsets() {
    let ceilings = over(FakeReads {
        access_group_servers: Scripted::Value(names(&["srv-group"])),
        toolset: Scripted::Value(tool_grants(&[("srv-set", &["tool-a"])])),
        end_user: Scripted::Value(Some(with_tools(
            with_groups(with_servers(row("perm-eu"), &["srv-a"]), &["g-1"]),
            "srv-tool",
            &["tool-a"],
        ))),
        ..FakeReads::standard()
    });
    assert_eq!(
        ceilings
            .end_user_servers(Some(&end_user_auth()))
            .await
            .unwrap(),
        names(&["srv-a", "srv-group", "srv-tool"])
    );
    assert!(!ceilings.reads.saw("toolsets:"));
}

#[tokio::test]
async fn end_user_servers_is_empty_without_a_permission() {
    let ceilings = over(FakeReads::standard());
    assert_eq!(
        ceilings
            .end_user_servers(Some(&end_user_auth()))
            .await
            .unwrap(),
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn end_user_servers_returns_the_getter_error_and_swallows_a_union_error() {
    let failing = over(FakeReads {
        end_user: Scripted::Unloadable,
        ..FakeReads::standard()
    });
    let error = failing
        .end_user_servers(Some(&end_user_auth()))
        .await
        .unwrap_err();
    assert!(matches!(error, CeilingsError::Unloadable(_)));
    let union = over(FakeReads {
        end_user: Scripted::Value(Some(row("perm-eu"))),
        access_group_servers: Scripted::Fault,
        ..FakeReads::standard()
    });
    assert_eq!(
        union
            .end_user_servers(Some(&end_user_auth()))
            .await
            .unwrap(),
        Vec::<String>::new()
    );
}
