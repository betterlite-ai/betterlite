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
    CountingCache, CountingClock, close, database, field, is_null, object, seed, strings, text,
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
    let team_absent_rows = WriterRows {
        store: database.store.clone(),
    }
    .fetch_rows(&without_team, &[RowKind::Team])
    .await
    .expect("the prefetch query");
    assert!(team_absent_rows.get(RowKind::Team).is_none());
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

    let membership_rows = WriterRows {
        store: database.store.clone(),
    }
    .fetch_rows(&all_refs(), &[RowKind::Membership])
    .await
    .expect("the prefetch query");
    assert!(membership_rows.get(RowKind::Membership).is_some());
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
    assert!(strings(user, "teams").is_empty(), "teams normalizes to []");
    assert!(
        strings(user, "models").is_empty(),
        "models normalizes to []"
    );
    assert!(
        strings(user, "allowed_cache_controls").is_empty(),
        "allowed_cache_controls normalizes to []"
    );
    assert!(
        strings(user, "policies").is_empty(),
        "policies normalizes to []"
    );
    assert!(
        strings(user, "organization_memberships").is_empty(),
        "organization_memberships normalizes to []"
    );
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
    assert!(
        strings(team, "admins").is_empty(),
        "admins normalizes to []"
    );
    assert!(
        strings(team, "members").is_empty(),
        "members normalizes to []"
    );
    assert!(
        strings(team, "models").is_empty(),
        "models normalizes to []"
    );
    assert!(
        strings(team, "team_member_permissions").is_empty(),
        "team_member_permissions normalizes to []"
    );
    assert!(
        strings(team, "access_group_ids").is_empty(),
        "access_group_ids normalizes to []"
    );
    assert!(
        strings(team, "policies").is_empty(),
        "policies normalizes to []"
    );
    assert!(
        strings(team, "default_team_member_models").is_empty(),
        "default_team_member_models normalizes to []"
    );
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
    let JsonData::Object(alias_map) = aliases else {
        panic!("the aliases are an object");
    };
    assert_eq!(
        alias_map.get("gpt"),
        Some(&JsonData::String("gpt-4".to_owned()))
    );
    let JsonData::Object(model_table) = object(team, "litellm_model_table") else {
        panic!("the model table is an object");
    };
    assert!(!model_table.contains_key("aliases"));
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
    let organization = rows
        .get(RowKind::Organization)
        .expect("the organization row");
    assert!(
        strings(organization, "models").is_empty(),
        "organization models normalize to []"
    );
    let project = rows.get(RowKind::Project).expect("the project row");
    assert!(
        strings(project, "models").is_empty(),
        "project models normalize to []"
    );
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
    let membership = rows.get(RowKind::Membership).expect("the membership row");
    let member_budget = object(membership, "litellm_budget_table");
    assert!(
        strings(&member_budget, "allowed_models").is_empty(),
        "membership allowed_models normalizes to []"
    );
    let organization = rows
        .get(RowKind::Organization)
        .expect("the organization row");
    let organization_budget = object(organization, "litellm_budget_table");
    assert!(
        strings(&organization_budget, "allowed_models").is_empty(),
        "organization allowed_models normalizes to []"
    );
    let project = rows.get(RowKind::Project).expect("the project row");
    let project_budget = object(project, "litellm_budget_table");
    assert!(
        strings(&project_budget, "allowed_models").is_empty(),
        "project allowed_models normalizes to []"
    );
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
    assert!(
        strings(&permission, "mcp_servers").is_empty(),
        "mcp_servers normalizes to []"
    );
    assert!(
        strings(&permission, "mcp_access_groups").is_empty(),
        "mcp_access_groups normalizes to []"
    );
    assert!(
        strings(&permission, "mcp_toolsets").is_empty(),
        "mcp_toolsets normalizes to []"
    );
    assert!(
        strings(&permission, "blocked_tools").is_empty(),
        "blocked_tools normalizes to []"
    );
    assert!(
        strings(&permission, "vector_stores").is_empty(),
        "vector_stores normalizes to []"
    );
    assert!(
        strings(&permission, "agents").is_empty(),
        "agents normalizes to []"
    );
    assert!(
        strings(&permission, "agent_access_groups").is_empty(),
        "agent_access_groups normalizes to []"
    );
    assert!(
        strings(&permission, "models").is_empty(),
        "models normalizes to []"
    );
    assert!(
        strings(&permission, "search_tools").is_empty(),
        "search_tools normalizes to []"
    );
    assert!(
        strings(&permission, "skills").is_empty(),
        "skills normalizes to []"
    );
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
        team_id: None,
        membership_user_id: None,
        organization_id: None,
        project_id: None,
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
    let team = writes
        .iter()
        .find(|write| write.0 == "team_id:team-1")
        .expect("the team write");
    let CacheValue::Object(fields) = &team.1 else {
        panic!("the team write holds an object");
    };
    assert_eq!(
        fields.get("last_refreshed_at"),
        Some(&CacheValue::Float(1234.5)),
        "the team write carries the query time"
    );
    let project = writes
        .iter()
        .find(|write| write.0 == "project_id:project-1")
        .expect("the project write");
    let CacheValue::Object(project_fields) = &project.1 else {
        panic!("the project write holds an object");
    };
    assert_eq!(
        project_fields.get("last_refreshed_at"),
        Some(&CacheValue::Float(1234.5)),
        "the project write carries the query time"
    );
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
