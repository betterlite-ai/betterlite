#![cfg(test)]

use std::sync::{Arc, Mutex};

use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_failures::gate::{DbLookupStallTracker, SystemClock};
use betterlite_db::database_replica::availability::Availability;
use tokio_util::task::TaskTracker;

use crate::auth_prefetch::db::{PrefetchDatabase, WriterRows};
use crate::auth_prefetch::entries::AuthObjectRefs;
use crate::auth_prefetch::orchestrator::OwnerPrefetch;
use crate::auth_prefetch::permission::store::{
    BoundedPermissionStore, ObjectPermissionStore, PermissionTable,
};
use crate::auth_prefetch::rows::RowKind;
use crate::auth_read::table::ReadContext;

mod support;

use support::{
    CountingCache, CountingClock, Database, close, database, field, is_null, object, seed, strings,
    text,
};

const FULL_SEED: &str = r#"
INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES
    ('budget-org', 'tester', 'tester'),
    ('budget-project', 'tester', 'tester'),
    ('budget-member', 'tester', 'tester');
INSERT INTO "LiteLLM_ObjectPermissionTable" (object_permission_id) VALUES
    ('perm-team'), ('perm-org'), ('perm-project');
INSERT INTO "LiteLLM_ModelTable" (id, aliases, created_by, updated_by)
    VALUES (7, '{"gpt": "gpt-4"}'::jsonb, 'tester', 'tester');
INSERT INTO "LiteLLM_OrganizationTable" (organization_id, organization_alias, budget_id, created_by, updated_by, object_permission_id)
    VALUES ('org-1', 'org one', 'budget-org', 'tester', 'tester', 'perm-org');
INSERT INTO "LiteLLM_UserTable" (user_id, teams, models, allowed_cache_controls, policies)
    VALUES ('user-1', ARRAY['team-b','team-a','team-b'], ARRAY['model-a'], NULL, NULL);
INSERT INTO "LiteLLM_TeamTable" (team_id, admins, members, models, model_id, object_permission_id)
    VALUES ('team-1', ARRAY['admin-1'], ARRAY['member-b','member-a','member-b'], ARRAY['model-a'], 7, 'perm-team');
INSERT INTO "LiteLLM_TeamMembership" (user_id, team_id, budget_id)
    VALUES ('user-1', 'team-1', 'budget-member');
INSERT INTO "LiteLLM_ProjectTable" (project_id, team_id, budget_id, models, object_permission_id, created_by, updated_by)
    VALUES ('project-1', 'team-1', 'budget-project', ARRAY['model-a'], 'perm-project', 'tester', 'tester');
"#;

fn all_refs() -> AuthObjectRefs {
    AuthObjectRefs {
        user_id: Some("user-1".to_owned()),
        team_id: Some("team-1".to_owned()),
        membership_user_id: Some("user-1".to_owned()),
        organization_id: Some("org-1".to_owned()),
        project_id: Some("project-1".to_owned()),
    }
}

fn all_kinds() -> [RowKind; 5] {
    [
        RowKind::User,
        RowKind::Team,
        RowKind::Membership,
        RowKind::Organization,
        RowKind::Project,
    ]
}

#[tokio::test]
async fn five_row_kinds_single_select() {
    let database = database().await;
    seed(&database.pool, FULL_SEED).await;
    let rows = WriterRows {
        store: database.store.clone(),
    }
    .fetch_rows(&all_refs(), &all_kinds())
    .await
    .expect("the prefetch query");
    let user = rows.get(RowKind::User).expect("the user row");
    let team = rows.get(RowKind::Team).expect("the team row");
    let membership = rows.get(RowKind::Membership).expect("the membership row");
    let organization = rows
        .get(RowKind::Organization)
        .expect("the organization row");
    let project = rows.get(RowKind::Project).expect("the project row");
    assert_eq!(text(user, "user_id"), "user-1");
    assert_eq!(strings(user, "teams"), ["team-b", "team-a", "team-b"]);
    assert_eq!(text(team, "team_id"), "team-1");
    assert_eq!(text(membership, "user_id"), "user-1");
    assert_eq!(text(membership, "team_id"), "team-1");
    assert_eq!(text(organization, "organization_id"), "org-1");
    assert_eq!(text(project, "project_id"), "project-1");

    let client = database.pool.get().await.expect("the database connection");
    let stored = client
        .query_one(
            r#"SELECT u.user_id::text, array_to_json(u.teams)::text FROM "LiteLLM_UserTable" u WHERE u.user_id = 'user-1'"#,
            &[],
        )
        .await
        .expect("the stored user row");
    let stored_id: String = stored.get(0);
    let stored_teams: String = stored.get(1);
    assert_eq!(stored_id, text(user, "user_id"));
    let stored_teams: JsonData =
        serde_json::from_str(&stored_teams).expect("the stored teams array");
    assert_eq!(&stored_teams, field(user, "teams"));
    drop(client);
    close(database).await;
}

#[tokio::test]
async fn absent_kinds_bind_sql_null() {
    let database = database().await;
    seed(&database.pool, FULL_SEED).await;
    let rows = WriterRows {
        store: database.store.clone(),
    }
    .fetch_rows(&all_refs(), &[RowKind::Team])
    .await
    .expect("the prefetch query");
    assert!(rows.get(RowKind::User).is_none());
    assert!(rows.get(RowKind::Team).is_some());

    let without_team = AuthObjectRefs {
        team_id: None,
        ..all_refs()
    };
    let rows = WriterRows {
        store: database.store.clone(),
    }
    .fetch_rows(&without_team, &[RowKind::Team])
    .await
    .expect("the prefetch query");
    assert!(rows.get(RowKind::Team).is_none());
    close(database).await;
}

#[tokio::test]
async fn membership_requires_team_parameter() {
    let database = database().await;
    seed(&database.pool, FULL_SEED).await;
    let without_team = AuthObjectRefs {
        team_id: None,
        ..all_refs()
    };
    let rows = WriterRows {
        store: database.store.clone(),
    }
    .fetch_rows(&without_team, &[RowKind::Membership])
    .await
    .expect("the prefetch query");
    assert!(rows.get(RowKind::Membership).is_none());

    let rows = WriterRows {
        store: database.store.clone(),
    }
    .fetch_rows(&all_refs(), &[RowKind::Membership])
    .await
    .expect("the prefetch query");
    assert!(rows.get(RowKind::Membership).is_some());
    close(database).await;
}

#[tokio::test]
async fn missing_scalar_rows_null() {
    let database = database().await;
    seed(
        &database.pool,
        r#"INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ('user-1');"#,
    )
    .await;
    let rows = WriterRows {
        store: database.store.clone(),
    }
    .fetch_rows(&all_refs(), &all_kinds())
    .await
    .expect("the prefetch query");
    assert!(rows.get(RowKind::User).is_some());
    assert!(rows.get(RowKind::Team).is_none());
    assert!(rows.get(RowKind::Membership).is_none());
    assert!(rows.get(RowKind::Organization).is_none());
    assert!(rows.get(RowKind::Project).is_none());
    close(database).await;
}

#[tokio::test]
async fn user_null_arrays_and_memberships() {
    let database = database().await;
    seed(
        &database.pool,
        r#"INSERT INTO "LiteLLM_UserTable" (user_id, teams, models, allowed_cache_controls, policies)
            VALUES ('user-1', NULL, NULL, NULL, NULL);"#,
    )
    .await;
    let rows = WriterRows {
        store: database.store.clone(),
    }
    .fetch_rows(&all_refs(), &[RowKind::User])
    .await
    .expect("the prefetch query");
    let user = rows.get(RowKind::User).expect("the user row");
    for list in [
        "teams",
        "models",
        "allowed_cache_controls",
        "policies",
        "organization_memberships",
    ] {
        assert!(strings(user, list).is_empty(), "{list} normalizes to []");
    }
    close(database).await;
}

#[tokio::test]
async fn team_all_null_arrays() {
    let database = database().await;
    seed(
        &database.pool,
        r#"INSERT INTO "LiteLLM_TeamTable" (
            team_id, admins, members, models, team_member_permissions,
            access_group_ids, policies, default_team_member_models
        ) VALUES ('team-1', NULL, NULL, NULL, NULL, NULL, NULL, NULL);"#,
    )
    .await;
    let rows = WriterRows {
        store: database.store.clone(),
    }
    .fetch_rows(&all_refs(), &[RowKind::Team])
    .await
    .expect("the prefetch query");
    let team = rows.get(RowKind::Team).expect("the team row");
    for list in [
        "admins",
        "members",
        "models",
        "team_member_permissions",
        "access_group_ids",
        "policies",
        "default_team_member_models",
    ] {
        assert!(strings(team, list).is_empty(), "{list} normalizes to []");
    }
    assert!(is_null(team, "litellm_model_table"));
    assert!(is_null(team, "object_permission"));
    close(database).await;
}

#[tokio::test]
async fn team_model_aliases_renamed() {
    let database = database().await;
    seed(
        &database.pool,
        r#"
INSERT INTO "LiteLLM_ModelTable" (id, aliases, created_by, updated_by)
    VALUES (7, '{"gpt": "gpt-4"}'::jsonb, 'tester', 'tester');
INSERT INTO "LiteLLM_TeamTable" (team_id, model_id) VALUES ('team-1', 7);
"#,
    )
    .await;
    let rows = WriterRows {
        store: database.store.clone(),
    }
    .fetch_rows(&all_refs(), &[RowKind::Team])
    .await
    .expect("the prefetch query");
    let team = rows.get(RowKind::Team).expect("the team row");
    let table = object(team, "litellm_model_table");
    let aliases = object(&table, "model_aliases");
    let JsonData::Object(aliases) = aliases else {
        panic!("the aliases are an object");
    };
    assert_eq!(
        aliases.get("gpt"),
        Some(&JsonData::String("gpt-4".to_owned()))
    );
    let JsonData::Object(table) = object(team, "litellm_model_table") else {
        panic!("the model table is an object");
    };
    assert!(!table.contains_key("aliases"));
    close(database).await;
}

#[tokio::test]
async fn nullable_related_rows_preserved() {
    let database = database().await;
    seed(
        &database.pool,
        r#"
INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by)
    VALUES ('budget-org', 'tester', 'tester'), ('budget-project', 'tester', 'tester');
INSERT INTO "LiteLLM_OrganizationTable" (organization_id, organization_alias, budget_id, created_by, updated_by)
    VALUES ('org-1', 'org one', 'budget-org', 'tester', 'tester');
INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ('user-1');
INSERT INTO "LiteLLM_TeamTable" (team_id) VALUES ('team-1');
INSERT INTO "LiteLLM_TeamMembership" (user_id, team_id) VALUES ('user-1', 'team-1');
INSERT INTO "LiteLLM_ProjectTable" (project_id, team_id, budget_id, created_by, updated_by)
    VALUES ('project-1', 'team-1', 'budget-project', 'tester', 'tester');
"#,
    )
    .await;
    let rows = WriterRows {
        store: database.store.clone(),
    }
    .fetch_rows(&all_refs(), &all_kinds())
    .await
    .expect("the prefetch query");
    assert!(is_null(
        rows.get(RowKind::Membership).expect("the membership"),
        "litellm_budget_table"
    ));
    assert!(is_null(
        rows.get(RowKind::Team).expect("the team"),
        "object_permission"
    ));
    assert!(is_null(
        rows.get(RowKind::Organization).expect("the organization"),
        "object_permission"
    ));
    assert!(is_null(
        rows.get(RowKind::Project).expect("the project"),
        "object_permission"
    ));
    close(database).await;
}

#[tokio::test]
async fn organization_project_null_models() {
    let database = database().await;
    seed(
        &database.pool,
        r#"
INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by)
    VALUES ('budget-org', 'tester', 'tester'), ('budget-project', 'tester', 'tester');
INSERT INTO "LiteLLM_OrganizationTable" (organization_id, organization_alias, budget_id, created_by, updated_by, models)
    VALUES ('org-1', 'org one', 'budget-org', 'tester', 'tester', NULL);
INSERT INTO "LiteLLM_TeamTable" (team_id) VALUES ('team-1');
INSERT INTO "LiteLLM_ProjectTable" (project_id, team_id, budget_id, created_by, updated_by, models)
    VALUES ('project-1', 'team-1', 'budget-project', 'tester', 'tester', NULL);
"#,
    )
    .await;
    let rows = WriterRows {
        store: database.store.clone(),
    }
    .fetch_rows(&all_refs(), &[RowKind::Organization, RowKind::Project])
    .await
    .expect("the prefetch query");
    for kind in [RowKind::Organization, RowKind::Project] {
        let row = rows.get(kind).expect("the row");
        assert!(strings(row, "models").is_empty(), "{kind:?} models");
    }
    close(database).await;
}

#[tokio::test]
async fn budget_allowed_models_null_to_empty() {
    let database = database().await;
    seed(
        &database.pool,
        r#"
INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by, allowed_models)
    VALUES ('budget-org', 'tester', 'tester', NULL),
           ('budget-project', 'tester', 'tester', NULL),
           ('budget-member', 'tester', 'tester', NULL);
INSERT INTO "LiteLLM_OrganizationTable" (organization_id, organization_alias, budget_id, created_by, updated_by)
    VALUES ('org-1', 'org one', 'budget-org', 'tester', 'tester');
INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ('user-1');
INSERT INTO "LiteLLM_TeamTable" (team_id) VALUES ('team-1');
INSERT INTO "LiteLLM_TeamMembership" (user_id, team_id, budget_id) VALUES ('user-1', 'team-1', 'budget-member');
INSERT INTO "LiteLLM_ProjectTable" (project_id, team_id, budget_id, created_by, updated_by)
    VALUES ('project-1', 'team-1', 'budget-project', 'tester', 'tester');
"#,
    )
    .await;
    let rows = WriterRows {
        store: database.store.clone(),
    }
    .fetch_rows(&all_refs(), &all_kinds())
    .await
    .expect("the prefetch query");
    for kind in [RowKind::Membership, RowKind::Organization, RowKind::Project] {
        let row = rows.get(kind).expect("the row");
        let budget = object(row, "litellm_budget_table");
        assert!(
            strings(&budget, "allowed_models").is_empty(),
            "{kind:?} allowed_models normalizes to []"
        );
    }
    close(database).await;
}

#[tokio::test]
async fn permission_each_null_list_to_empty() {
    let database = database().await;
    seed(
        &database.pool,
        r#"
INSERT INTO "LiteLLM_ObjectPermissionTable" (
    object_permission_id, mcp_servers, mcp_access_groups, mcp_toolsets, blocked_tools,
    vector_stores, agents, agent_access_groups, models, search_tools, skills
) VALUES ('perm-team', NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL);
INSERT INTO "LiteLLM_TeamTable" (team_id, object_permission_id)
    VALUES ('team-1', 'perm-team');
"#,
    )
    .await;
    let rows = WriterRows {
        store: database.store.clone(),
    }
    .fetch_rows(&all_refs(), &[RowKind::Team])
    .await
    .expect("the prefetch query");
    let team = rows.get(RowKind::Team).expect("the team row");
    let permission = object(team, "object_permission");
    for list in [
        "mcp_servers",
        "mcp_access_groups",
        "mcp_toolsets",
        "blocked_tools",
        "vector_stores",
        "agents",
        "agent_access_groups",
        "models",
        "search_tools",
        "skills",
    ] {
        assert!(
            strings(&permission, list).is_empty(),
            "{list} normalizes to []"
        );
    }
    close(database).await;
}

#[tokio::test]
async fn arrays_keep_order_and_duplicates() {
    let database = database().await;
    seed(
        &database.pool,
        r#"
INSERT INTO "LiteLLM_ObjectPermissionTable" (object_permission_id, models)
    VALUES ('perm-team', ARRAY['model-b','model-a','model-b']);
INSERT INTO "LiteLLM_UserTable" (user_id, teams)
    VALUES ('user-1', ARRAY['team-b','team-a','team-b']);
INSERT INTO "LiteLLM_TeamTable" (team_id, members, object_permission_id)
    VALUES ('team-1', ARRAY['member-b','member-a','member-b'], 'perm-team');
"#,
    )
    .await;
    let rows = WriterRows {
        store: database.store.clone(),
    }
    .fetch_rows(&all_refs(), &[RowKind::User, RowKind::Team])
    .await
    .expect("the prefetch query");
    assert_eq!(
        strings(rows.get(RowKind::User).expect("the user"), "teams"),
        ["team-b", "team-a", "team-b"]
    );
    let team = rows.get(RowKind::Team).expect("the team");
    assert_eq!(
        strings(team, "members"),
        ["member-b", "member-a", "member-b"]
    );
    let permission = object(team, "object_permission");
    assert_eq!(
        strings(&permission, "models"),
        ["model-b", "model-a", "model-b"]
    );
    close(database).await;
}

#[tokio::test]
async fn raw_prefetch_uses_writer() {
    let writer = database().await;
    let reader = database().await;
    seed(
        &writer.pool,
        r#"INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ('user-1');"#,
    )
    .await;
    seed(
        &reader.pool,
        r#"INSERT INTO "LiteLLM_UserTable" (user_id) VALUES ('user-2');"#,
    )
    .await;
    let refs = AuthObjectRefs {
        user_id: Some("user-1".to_owned()),
        ..AuthObjectRefs::default()
    };
    let writer_rows = WriterRows {
        store: writer.store.clone(),
    };
    let reader_rows = WriterRows {
        store: reader.store.clone(),
    };
    let found = writer_rows
        .fetch_rows(&refs, &[RowKind::User])
        .await
        .expect("the writer query");
    assert_eq!(
        text(found.get(RowKind::User).expect("the user row"), "user_id"),
        "user-1"
    );
    let missing = reader_rows
        .fetch_rows(&refs, &[RowKind::User])
        .await
        .expect("the other database query");
    assert!(missing.get(RowKind::User).is_none());
    close(writer).await;
    close(reader).await;
}

#[tokio::test]
async fn one_timestamp_after_query() {
    let database = database().await;
    seed(&database.pool, FULL_SEED).await;
    let cache = CountingCache::default();
    let clock = CountingClock::reading(1234.5);
    let rows = WriterRows {
        store: database.store.clone(),
    };
    let owner = OwnerPrefetch {
        cache: &cache,
        database: Some(&rows),
        clock: &clock,
        organization_ttl: 7.0,
    };
    owner.prefetch(&all_refs()).await;
    assert_eq!(clock.calls(), 1);
    let writes = cache.writes();
    for key in ["team_id:team-1", "project_id:project-1"] {
        let write = writes
            .iter()
            .find(|write| write.0 == key)
            .unwrap_or_else(|| panic!("the {key} write"));
        let CacheValue::Object(fields) = &write.1 else {
            panic!("{key} holds an object");
        };
        assert_eq!(
            fields.get("last_refreshed_at"),
            Some(&CacheValue::Float(1234.5)),
            "{key} carries the query time"
        );
    }
    close(database).await;
}

#[tokio::test]
async fn permission_table_reads_seeded_row() {
    let database = database().await;
    seed(
        &database.pool,
        r#"INSERT INTO "LiteLLM_ObjectPermissionTable" (object_permission_id, models)
            VALUES ('perm-1', ARRAY['model-a']);"#,
    )
    .await;
    let table = Arc::new(PermissionTable {
        reader: database.store.clone(),
        writer: database.store.clone(),
        availability: Arc::new(Availability::default()),
        use_writer: true,
    });
    let store = BoundedPermissionStore {
        table,
        context: ReadContext {
            default_deadline_seconds: 5.0,
            tracker: Arc::new(DbLookupStallTracker {
                clock: SystemClock,
                last_hit: Mutex::new(None),
            }),
            tasks: TaskTracker::new(),
        },
    };
    let row = store
        .find_permission("perm-1")
        .await
        .expect("the permission lookup")
        .expect("the stored permission");
    assert_eq!(text(&row, "object_permission_id"), "perm-1");
    assert_eq!(strings(&row, "models"), ["model-a"]);
    assert!(
        store
            .find_permission("perm-missing")
            .await
            .expect("the missing lookup")
            .is_none()
    );
    close(database).await;
}
