use crate::auth_limit_inputs::tests::support;

#[test]
fn account_limits_readers() {
    let auth = support::auth(&[
        ("api_key", support::text("sk-hashed")),
        ("org_id", support::text("org-1")),
        ("team_id", support::text("team-1")),
        ("project_id", support::text("project-1")),
        ("organization_rpm_limit", support::number(100)),
        ("organization_tpm_limit", support::number(1000)),
    ]);
    let limits = auth.account_limits();
    assert!(limits.hashed_api_key.is_some_and(|key| !key.is_empty()));
    assert_eq!(limits.org_id, Some("org-1"));
    assert_eq!(limits.team_id, Some("team-1"));
    assert_eq!(limits.project_id, Some("project-1"));
    assert_eq!(limits.organization_rpm_limit, Some(100));
    assert_eq!(limits.organization_tpm_limit, Some(1000));
}

#[test]
fn account_limits_absent_fields_are_none() {
    let auth = support::auth(&[]);
    let limits = auth.account_limits();
    assert_eq!(limits.hashed_api_key, None);
    assert_eq!(limits.org_id, None);
    assert_eq!(limits.team_id, None);
    assert_eq!(limits.project_id, None);
    assert_eq!(limits.organization_rpm_limit, None);
    assert_eq!(limits.organization_tpm_limit, None);
}
