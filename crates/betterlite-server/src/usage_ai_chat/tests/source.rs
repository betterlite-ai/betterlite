#[tokio::test]
async fn source_global_builds_the_user_scope_and_returns_the_page() {
    let expected = empty_page();
    let source = DailyActivityUsageData {
        repository: Some(recording_repository().serving(expected.clone())),
    };
    let result = source
        .global("2026-01-01", "2026-01-31", Some("alice"))
        .await
        .unwrap();
    assert_eq!(result, expected);
    let calls = source.repository.as_ref().unwrap().aggregated_calls();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call.table, DailyActivityTable::User);
    assert_eq!(call.entity_id_field, ENTITY_FIELD_USER);
    assert_eq!(call.entity_ids, Some(vec!["alice".to_owned()]));
    assert_eq!(call.api_keys, None);
    assert!(call.exclude_entity_ids.is_empty());
    assert_eq!(call.start_date, "2026-01-01");
    assert_eq!(call.end_date, "2026-01-31");
    assert_eq!(call.model, None);
    assert_eq!(call.timezone_offset_minutes, None);
    assert!(!call.include_current_utc_day);
}

#[tokio::test]
async fn source_global_without_a_user_id_has_no_entity_filter() {
    let source = DailyActivityUsageData {
        repository: Some(recording_repository().serving(empty_page())),
    };
    source
        .global("2026-01-01", "2026-01-31", None)
        .await
        .unwrap();
    let calls = source.repository.as_ref().unwrap().aggregated_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].entity_ids, None);
}

#[tokio::test]
async fn source_teams_pages_first_with_the_team_table() {
    let source = DailyActivityUsageData {
        repository: Some(recording_repository().serving(empty_page())),
    };
    let ids = vec!["team-1".to_owned()];
    source
        .teams("2026-02-01", "2026-02-28", Some(ids.as_slice()))
        .await
        .unwrap();
    let calls = source.repository.as_ref().unwrap().paged_calls();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call.table, DailyActivityTable::Team);
    assert_eq!(call.entity_id_field, ENTITY_FIELD_TEAM);
    assert_eq!(call.entity_id, Some(ScopeIds::Many(ids.clone())));
    assert_eq!(call.start_date.as_deref(), Some("2026-02-01"));
    assert_eq!(call.end_date.as_deref(), Some("2026-02-28"));
    assert_eq!(call.model, None);
    assert_eq!(call.api_key, None);
    assert_eq!(call.page, 1);
    assert_eq!(call.page_size, PAGINATED_PAGE_SIZE);
    assert_eq!(call.timezone_offset_minutes, None);
    assert!(!call.include_current_utc_day);
    assert_eq!(call.extras, ExtrasAbsence::Both);
    assert!(call.exclude_entity_ids.is_empty());
}

#[tokio::test]
async fn source_tags_keep_no_filter_apart_from_an_empty_list() {
    let source = DailyActivityUsageData {
        repository: Some(
            recording_repository()
                .serving(empty_page())
                .serving(empty_page()),
        ),
    };
    source.tags("2026-03-01", "2026-03-31", None).await.unwrap();
    let empty: Vec<String> = Vec::new();
    source
        .tags("2026-03-01", "2026-03-31", Some(empty.as_slice()))
        .await
        .unwrap();
    let calls = source.repository.as_ref().unwrap().paged_calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].table, DailyActivityTable::Tag);
    assert_eq!(calls[0].entity_id_field, ENTITY_FIELD_TAG);
    assert_eq!(calls[0].entity_id, None);
    assert_eq!(calls[1].entity_id, Some(ScopeIds::Many(Vec::new())));
    assert_eq!(calls[1].page, 1);
    assert_eq!(calls[1].page_size, PAGINATED_PAGE_SIZE);
}

#[tokio::test]
async fn source_without_a_repository_fails_with_the_database_error() {
    let source = DailyActivityUsageData::<RecordingRepository> { repository: None };
    let error = source
        .global("2026-01-01", "2026-01-31", None)
        .await
        .unwrap_err();
    assert!(matches!(error, UsageChatError::DatabaseNotConnected));
    assert_eq!(
        error.to_string(),
        CommonProxyErrors::DbNotConnectedError.as_str()
    );
    let teams = source
        .teams("2026-01-01", "2026-01-31", None)
        .await
        .unwrap_err();
    assert!(matches!(teams, UsageChatError::DatabaseNotConnected));
    assert_eq!(
        teams.to_string(),
        CommonProxyErrors::DbNotConnectedError.as_str()
    );
    let tags = source
        .tags("2026-01-01", "2026-01-31", None)
        .await
        .unwrap_err();
    assert!(matches!(tags, UsageChatError::DatabaseNotConnected));
    assert_eq!(
        tags.to_string(),
        CommonProxyErrors::DbNotConnectedError.as_str()
    );
}

#[test]
fn source_constants_match_litellm() {
    assert_eq!(
        TABLE_DAILY_USER_SPEND,
        DailyActivityTable::User.metadata().table_name
    );
    assert_eq!(
        TABLE_DAILY_TEAM_SPEND,
        DailyActivityTable::Team.metadata().table_name
    );
    assert_eq!(
        TABLE_DAILY_TAG_SPEND,
        DailyActivityTable::Tag.metadata().table_name
    );
    assert_eq!(
        ENTITY_FIELD_USER,
        DailyActivityTable::User.metadata().entity_field
    );
    assert_eq!(
        ENTITY_FIELD_TEAM,
        DailyActivityTable::Team.metadata().entity_field
    );
    assert_eq!(
        ENTITY_FIELD_TAG,
        DailyActivityTable::Tag.metadata().entity_field
    );
    assert_eq!(PAGINATED_PAGE_SIZE, 200);
    assert_eq!(MAX_CHAT_MESSAGES, 20);
    assert_eq!(USAGE_TOP_API_KEYS_DEFAULT, 100);
    assert_eq!(USAGE_AI_TEMPERATURE.to_bits(), 0.2_f64.to_bits());
    assert_eq!(TOP_N_MODELS, 15);
    assert_eq!(TOP_N_PROVIDERS, 10);
    assert_eq!(DEFAULT_CHAT_MODEL, "gpt-4o-mini");
}

#[tokio::test]
async fn source_global_adapter_issues_one_aggregated_query() {
    let source = DailyActivityUsageData {
        repository: Some(concrete_repository(recording_database())),
    };
    let page = source
        .global("2026-01-01", "2026-01-31", Some("alice"))
        .await
        .unwrap();
    assert!(page.results.is_empty());
    assert_eq!(
        page.metadata.api_key_limit,
        Some(USAGE_TOP_API_KEYS_DEFAULT)
    );
    assert_eq!(
        source.repository.as_ref().unwrap().database.queries().len(),
        1
    );
}

#[tokio::test]
async fn source_team_adapter_pages_with_skip_zero_and_take_two_hundred() {
    let source = DailyActivityUsageData {
        repository: Some(concrete_repository(recording_database())),
    };
    let ids = vec!["team-1".to_owned()];
    source
        .teams("2026-02-01", "2026-02-28", Some(ids.as_slice()))
        .await
        .unwrap();
    let database = &source.repository.as_ref().unwrap().database;
    assert_eq!(database.counts(), [DailyActivityTable::Team]);
    let finds = database.finds();
    assert_eq!(finds.len(), 1);
    assert_eq!(finds[0].table, DailyActivityTable::Team);
    assert_eq!(finds[0].skip, 0);
    assert_eq!(finds[0].take, PAGINATED_PAGE_SIZE);
}
