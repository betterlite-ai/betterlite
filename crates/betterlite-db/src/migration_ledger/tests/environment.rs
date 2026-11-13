#[test]
fn direct_schema_preserved() {
    let effective = migration_environment(
        Some("postgresql://writer/db?schema=tenant_a&sslmode=require"),
        Some("postgresql://direct:6543/db?pgbouncer=true&schema=pooled&connection_limit=5"),
    )
    .expect("the URLs parse");
    assert_eq!(
        effective.as_deref(),
        Some("postgresql://direct:6543/db?pgbouncer=true&connection_limit=5&schema=tenant_a")
    );
}

#[test]
fn public_schema_default() {
    let effective = migration_environment(
        Some("postgresql://writer/db"),
        Some("postgresql://direct/db?connection_limit=2"),
    )
    .expect("the URLs parse");
    assert_eq!(
        effective.as_deref(),
        Some("postgresql://direct/db?connection_limit=2&schema=public")
    );
    let blank_schema = migration_environment(
        Some("postgresql://writer/db?schema="),
        Some("postgresql://direct/db"),
    )
    .expect("the URLs parse");
    assert_eq!(
        blank_schema.as_deref(),
        Some("postgresql://direct/db?schema=public")
    );
}

#[test]
fn direct_url_absent_passthrough() {
    let database = "postgresql://writer/db?schema=tenant_a";
    assert_eq!(
        migration_environment(Some(database), None).expect("the URL parses"),
        Some(database.to_owned())
    );
    assert_eq!(
        migration_environment(Some(database), Some("")).expect("the URL parses"),
        Some(database.to_owned())
    );
    assert_eq!(
        migration_environment(None, Some("postgresql://direct/db")).expect("nothing to parse"),
        None
    );
    assert_eq!(
        prisma_schema_param("postgresql://writer/db?schema=first&schema=second")
            .expect("the URL parses"),
        Some("first".to_owned())
    );
}
