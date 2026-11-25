// the tracing span a caller carries is no part of its MCP identity, so a valid auth with an opaque span still decodes
#[test]
fn decode_reads_an_opaque_span_credential() {
    let record = decode::<StoreDown>(Some(&opaque_span_auth())).unwrap();
    assert_eq!(record.user_id.as_deref(), Some("user-1"));
    assert_eq!(record.team_id.as_deref(), Some("team-1"));
}

#[tokio::test]
async fn user_ceiling_reads_an_opaque_span_credential() {
    let ceilings = over(FakeReads {
        user: Scripted::Fault,
        ..FakeReads::standard()
    });
    let error = ceilings
        .apply_user_server_ceiling(
            names(&["srv-a"]),
            Some(&opaque_span_auth()),
            CeilingSource::Key,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        CeilingsError::UserCeilingUnresolvable {
            user_id: Some(user_id)
        } if user_id == "user-1"
    ));
}

#[tokio::test]
async fn agent_caller_ceiling_reads_an_opaque_span_caller() {
    let ceilings = over(FakeReads {
        caller: Scripted::Value(Some(opaque_span_auth())),
        team_servers: Scripted::Value(names(&["srv-a"])),
        ..FakeReads::standard()
    });
    let result = ceilings
        .apply_agent_caller_ceiling(names(&["srv-a", "srv-b"]), Some(&agent_auth()))
        .await
        .unwrap();
    assert_eq!(result, capped(&["srv-a"], true));
    assert!(ceilings.reads.saw("team_servers:user-1"));
}
