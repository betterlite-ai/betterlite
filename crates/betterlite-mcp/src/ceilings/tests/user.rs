#[tokio::test]
async fn user_servers_without_a_permission_is_no_restriction() {
    let ceilings = over(FakeReads::standard());
    assert_eq!(
        ceilings.user_servers(Some(&user_auth())).await,
        CeilingServers::NoRestriction
    );
}

#[tokio::test]
async fn user_servers_includes_toolset_servers() {
    let ceilings = over(FakeReads {
        store: StoreState {
            database: true,
            fresh: true,
        },
        toolset: Scripted::Value(tool_grants(&[("srv-set", &["tool-a"])])),
        user: Scripted::Value(Some(with_servers(row("perm-user"), &["srv-a"]))),
        ..FakeReads::standard()
    });
    assert_eq!(
        ceilings.user_servers(Some(&user_auth())).await,
        CeilingServers::Servers(names(&["srv-a", "srv-set"]))
    );
    assert!(ceilings.reads.saw("toolsets:true"));
}

#[tokio::test]
async fn user_servers_turns_every_fault_into_unresolved() {
    let resolved = over(FakeReads {
        user: Scripted::Value(Some(row("perm-user"))),
        toolset: Scripted::Unloadable,
        ..FakeReads::standard()
    });
    assert_eq!(
        resolved.user_servers(Some(&user_auth())).await,
        CeilingServers::Unresolved
    );
    let failed = over(FakeReads {
        user: Scripted::Fault,
        ..FakeReads::standard()
    });
    assert_eq!(
        failed.user_servers(Some(&user_auth())).await,
        CeilingServers::Unresolved
    );
    let unloadable = over(FakeReads {
        user: Scripted::Unloadable,
        ..FakeReads::standard()
    });
    assert_eq!(
        unloadable.user_servers(Some(&user_auth())).await,
        CeilingServers::Unresolved
    );
}

#[tokio::test]
async fn user_ceiling_skips_a_keyless_source_without_reading() {
    let ceilings = over(FakeReads::standard());
    let result = ceilings
        .apply_user_server_ceiling(
            names(&["srv-a"]),
            Some(&user_auth()),
            CeilingSource::Keyless,
        )
        .await
        .unwrap();
    assert_eq!(result, capped(&["srv-a"], false));
    assert!(ceilings.reads.calls().is_empty());
}

#[tokio::test]
async fn user_ceiling_errors_on_an_unresolved_entitlement() {
    let failed = over(FakeReads {
        user: Scripted::Fault,
        ..FakeReads::standard()
    });
    let error = failed
        .apply_user_server_ceiling(names(&["srv-a"]), Some(&user_auth()), CeilingSource::Key)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        CeilingsError::UserCeilingUnresolvable { user_id: Some(user_id) } if user_id == "user-1"
    ));
    let unloadable = over(FakeReads {
        user: Scripted::Unloadable,
        ..FakeReads::standard()
    });
    let refused = unloadable
        .apply_user_server_ceiling(names(&["srv-a"]), Some(&user_auth()), CeilingSource::Key)
        .await
        .unwrap_err();
    assert!(matches!(
        refused,
        CeilingsError::UserCeilingUnresolvable { .. }
    ));
}

#[tokio::test]
async fn user_ceiling_leaves_the_list_when_the_user_restricts_nothing() {
    let ceilings = over(FakeReads::standard());
    let allowed = names(&["srv-a"]);
    let unchanged = capped(&["srv-a"], false);
    assert_eq!(
        ceilings
            .apply_user_server_ceiling(allowed.clone(), Some(&user_auth()), CeilingSource::Key)
            .await
            .unwrap(),
        unchanged
    );
    let empty = over(FakeReads {
        user: Scripted::Value(Some(row("perm-user"))),
        ..FakeReads::standard()
    });
    assert_eq!(
        empty
            .apply_user_server_ceiling(allowed, Some(&user_auth()), CeilingSource::Key)
            .await
            .unwrap(),
        unchanged
    );
}

#[tokio::test]
async fn user_ceiling_caps_by_the_user_list() {
    let ceilings = over(FakeReads {
        user: Scripted::Value(Some(with_servers(row("perm-user"), &["srv-a"]))),
        ..FakeReads::standard()
    });
    let result = ceilings
        .apply_user_server_ceiling(
            names(&["srv-a", "srv-b"]),
            Some(&user_auth()),
            CeilingSource::Key,
        )
        .await
        .unwrap();
    assert_eq!(result, capped(&["srv-a"], true));
}

#[tokio::test]
async fn user_places_ceiling_answers_each_shape() {
    let unrestricted = over(FakeReads::standard());
    assert!(!unrestricted.user_places_ceiling(Some(&user_auth())).await);
    let empty = over(FakeReads {
        user: Scripted::Value(Some(row("perm-user"))),
        ..FakeReads::standard()
    });
    assert!(!empty.user_places_ceiling(Some(&user_auth())).await);
    let bounded = over(FakeReads {
        user: Scripted::Value(Some(with_servers(row("perm-user"), &["srv-a"]))),
        ..FakeReads::standard()
    });
    assert!(bounded.user_places_ceiling(Some(&user_auth())).await);
    let unresolved = over(FakeReads {
        user: Scripted::Fault,
        ..FakeReads::standard()
    });
    assert!(unresolved.user_places_ceiling(Some(&user_auth())).await);
}

#[tokio::test]
async fn user_admin_view_unscoped_requires_an_admin_credential() {
    let ceilings = over(FakeReads::standard());
    assert!(!ceilings.admin_view_unscoped(None).await);
    let plain = auth_from_json(r#"{"user_id":"user-1","user_role":"internal_user"}"#);
    assert!(!ceilings.admin_view_unscoped(Some(&plain)).await);
    let explicit = over(FakeReads {
        marks: AdmissionMarks {
            admitted_user_subject: false,
            explicit_grants_only: true,
        },
        ..FakeReads::standard()
    });
    assert!(!explicit.admin_view_unscoped(Some(&admin_auth())).await);
}

#[tokio::test]
async fn user_admin_view_unscoped_rejects_a_scoped_credential() {
    let scoped = auth_from_json(
        r#"{"user_id":"admin-1","user_role":"proxy_admin","object_permission":{"object_permission_id":"perm-key","mcp_servers":[]}}"#,
    );
    let ceilings = over(FakeReads::standard());
    assert!(!ceilings.admin_view_unscoped(Some(&scoped)).await);
    let admitted = over(FakeReads {
        marks: AdmissionMarks {
            admitted_user_subject: true,
            explicit_grants_only: false,
        },
        ..FakeReads::standard()
    });
    assert!(admitted.admin_view_unscoped(Some(&scoped)).await);
}

#[tokio::test]
async fn user_admin_view_unscoped_follows_the_user_ceiling() {
    let ceilings = over(FakeReads::standard());
    assert!(ceilings.admin_view_unscoped(Some(&admin_auth())).await);
    let bounded = over(FakeReads {
        user: Scripted::Value(Some(with_servers(row("perm-user"), &["srv-a"]))),
        ..FakeReads::standard()
    });
    assert!(!bounded.admin_view_unscoped(Some(&admin_auth())).await);
    let unresolved = over(FakeReads {
        user: Scripted::Fault,
        ..FakeReads::standard()
    });
    assert!(!unresolved.admin_view_unscoped(Some(&admin_auth())).await);
}
