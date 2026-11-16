#[test]
fn parse_ids_none_or_empty_is_none() {
    assert_eq!(parse_csv_ids(None), None);
    assert_eq!(parse_csv_ids(Some("")), None);
}

#[test]
fn parse_ids_trims_and_drops_empty_parts() {
    assert_eq!(
        parse_csv_ids(Some(" team-1 , team-2 ,, ")),
        Some(vec!["team-1".to_owned(), "team-2".to_owned()])
    );
}

#[test]
fn parse_ids_only_commas_is_an_empty_list() {
    assert_eq!(parse_csv_ids(Some(" , ")), Some(Vec::new()));
}

#[test]
fn fetch_args_non_admin_uses_the_callers_id() {
    let arguments = args(&[
        ("start_date", "2026-01-01"),
        ("end_date", "2026-01-31"),
        ("user_id", "mallory"),
    ]);
    assert_eq!(
        resolve_fetch_kwargs(UsageTool::Usage, &arguments, Some("alice"), false).unwrap(),
        FetchArguments::Global {
            start_date: "2026-01-01".to_owned(),
            end_date: "2026-01-31".to_owned(),
            user_id: Some("alice".to_owned()),
        }
    );
}

#[test]
fn fetch_args_non_admin_without_a_caller_id_is_refused() {
    let arguments = args(&[("start_date", "2026-01-01"), ("end_date", "2026-01-31")]);
    let error = resolve_fetch_kwargs(UsageTool::Usage, &arguments, None, false).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Non-admin caller has user_id=None; refusing to issue an unscoped query. Endpoint-level guard missing."
    );
}

#[test]
fn fetch_args_admin_forwards_only_a_non_empty_user_id() {
    let blank = args(&[
        ("start_date", "2026-01-01"),
        ("end_date", "2026-01-31"),
        ("user_id", ""),
    ]);
    assert_eq!(
        resolve_fetch_kwargs(UsageTool::Usage, &blank, Some("admin-1"), true).unwrap(),
        FetchArguments::Global {
            start_date: "2026-01-01".to_owned(),
            end_date: "2026-01-31".to_owned(),
            user_id: None,
        }
    );
    let targeted = args(&[
        ("start_date", "2026-01-01"),
        ("end_date", "2026-01-31"),
        ("user_id", "target"),
    ]);
    assert_eq!(
        resolve_fetch_kwargs(UsageTool::Usage, &targeted, Some("admin-1"), true).unwrap(),
        FetchArguments::Global {
            start_date: "2026-01-01".to_owned(),
            end_date: "2026-01-31".to_owned(),
            user_id: Some("target".to_owned()),
        }
    );
}

#[rstest]
#[case::usage(UsageTool::Usage)]
#[case::team(UsageTool::Team)]
#[case::tag(UsageTool::Tag)]
fn fetch_args_missing_or_empty_dates_fail_for_every_tool(#[case] tool: UsageTool) {
    let empty = args(&[]);
    assert_eq!(
        resolve_fetch_kwargs(tool, &empty, Some("alice"), true)
            .unwrap_err()
            .to_string(),
        "Missing required start_date or end_date from tool arguments"
    );
    let missing_end = args(&[("start_date", "2026-01-01")]);
    assert_eq!(
        resolve_fetch_kwargs(tool, &missing_end, Some("alice"), true)
            .unwrap_err()
            .to_string(),
        "Missing required start_date or end_date from tool arguments"
    );
    let blank_end = args(&[("start_date", "2026-01-01"), ("end_date", "")]);
    assert_eq!(
        resolve_fetch_kwargs(tool, &blank_end, Some("alice"), true)
            .unwrap_err()
            .to_string(),
        "Missing required start_date or end_date from tool arguments"
    );
}

#[test]
fn fetch_args_team_and_tag_pass_only_non_empty_filters() {
    let with_ids = args(&[
        ("start_date", "2026-01-01"),
        ("end_date", "2026-01-31"),
        ("team_ids", "team-1, team-2"),
    ]);
    assert_eq!(
        resolve_fetch_kwargs(UsageTool::Team, &with_ids, None, true).unwrap(),
        FetchArguments::Team {
            start_date: "2026-01-01".to_owned(),
            end_date: "2026-01-31".to_owned(),
            team_ids: Some("team-1, team-2".to_owned()),
        }
    );
    let blank_ids = args(&[
        ("start_date", "2026-01-01"),
        ("end_date", "2026-01-31"),
        ("team_ids", ""),
    ]);
    assert_eq!(
        resolve_fetch_kwargs(UsageTool::Team, &blank_ids, None, true).unwrap(),
        FetchArguments::Team {
            start_date: "2026-01-01".to_owned(),
            end_date: "2026-01-31".to_owned(),
            team_ids: None,
        }
    );
    let with_tags = args(&[
        ("start_date", "2026-01-01"),
        ("end_date", "2026-01-31"),
        ("tags", "prod,beta"),
    ]);
    assert_eq!(
        resolve_fetch_kwargs(UsageTool::Tag, &with_tags, None, true).unwrap(),
        FetchArguments::Tag {
            start_date: "2026-01-01".to_owned(),
            end_date: "2026-01-31".to_owned(),
            tags: Some("prod,beta".to_owned()),
        }
    );
    let blank_tags = args(&[
        ("start_date", "2026-01-01"),
        ("end_date", "2026-01-31"),
        ("tags", ""),
    ]);
    assert_eq!(
        resolve_fetch_kwargs(UsageTool::Tag, &blank_tags, None, true).unwrap(),
        FetchArguments::Tag {
            start_date: "2026-01-01".to_owned(),
            end_date: "2026-01-31".to_owned(),
            tags: None,
        }
    );
}

#[test]
fn fetch_args_decode_keeps_key_order_and_rejects_other_shapes() {
    let decoded =
        decode_tool_arguments(r#"{"end_date": "2026-01-31", "start_date": "2026-01-01"}"#).unwrap();
    assert_eq!(
        decoded.keys().map(String::as_str).collect::<Vec<_>>(),
        ["end_date", "start_date"]
    );
    assert_eq!(decoded["start_date"], "2026-01-01");
    assert!(decode_tool_arguments("[1, 2]").is_err());
    assert!(decode_tool_arguments(r#"{"start_date": 5}"#).is_err());
    assert!(decode_tool_arguments(r#"{"start_date": null}"#).is_err());
    assert!(decode_tool_arguments("").is_err());
    assert!(decode_tool_arguments(r#"{"start_date": "2026-01-01"#).is_err());
}
