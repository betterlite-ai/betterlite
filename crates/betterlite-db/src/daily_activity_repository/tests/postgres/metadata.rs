use super::support::{repository, run};
use std::collections::BTreeSet;

#[tokio::test]
async fn postgres_active_null_metadata_no_archive_fallback() {
    run(|pool| async move {
        let client = pool.get().await.unwrap();
        let repository = repository(&client, None);
        client
            .batch_execute(
                "INSERT INTO \"LiteLLM_VerificationToken\" (token, key_alias, team_id, user_id, metadata) VALUES
                    ('sk-null', 'active-alias', 'team-1', 'user-1', 'null'::jsonb),
                    ('sk-tags', 'tagged', NULL, NULL, '{\"tags\": [\"a\", \"b\"]}'::jsonb),
                    ('sk-bad-tags', NULL, NULL, NULL, '{\"tags\": [\"a\", 1]}'::jsonb);
                 INSERT INTO \"LiteLLM_DeletedVerificationToken\" (id, token, key_alias, metadata, deleted_at) VALUES
                    ('d1', 'sk-null', 'archived-alias', '{\"tags\": [\"stale\"]}'::jsonb, '2026-03-01'),
                    ('d2', 'sk-gone', 'gone-old', '{\"tags\": [\"old\"]}'::jsonb, '2026-01-01'),
                    ('d3', 'sk-gone', 'gone-new', '{\"tags\": [\"new\"]}'::jsonb, '2026-02-01');",
            )
            .await
            .unwrap();
        let wanted: BTreeSet<String> = [
            "sk-null",
            "sk-tags",
            "sk-bad-tags",
            "sk-gone",
            "sk-absent",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        repository.key_metadata(&wanted, None).await.unwrap();
        let call = repository.proxy_reads.last_call();
        assert_eq!(call.api_keys, wanted);
        assert!(call.window.is_none());
        let found: Vec<&str> = call.resolved.keys().map(String::as_str).collect();
        assert_eq!(found, ["sk-bad-tags", "sk-gone", "sk-null", "sk-tags"]);

        let null_metadata = &call.resolved["sk-null"];
        assert!(null_metadata.key_exists);
        assert_eq!(null_metadata.key_alias.as_deref(), Some("active-alias"));
        assert_eq!(null_metadata.team_id.as_deref(), Some("team-1"));
        assert_eq!(null_metadata.user_id.as_deref(), Some("user-1"));
        assert!(null_metadata.tags.is_empty());

        let tagged = &call.resolved["sk-tags"];
        assert!(tagged.key_exists);
        assert_eq!(tagged.tags, ["a", "b"]);

        assert!(call.resolved["sk-bad-tags"].tags.is_empty());

        let archived = &call.resolved["sk-gone"];
        assert!(!archived.key_exists);
        assert_eq!(archived.key_alias.as_deref(), Some("gone-new"));
        assert_eq!(archived.tags, ["new"]);
    })
    .await;
}
