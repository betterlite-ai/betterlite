#[test]
fn summary_usage_prints_the_header_and_no_data_lists() {
    let text = summarise_usage_data(&page(Vec::new(), metadata("12.5", 7, 5, 2, 300))).unwrap();
    assert_eq!(
        text,
        concat!(
            "Total Spend: $12.5000\n",
            "Total Requests: 7\n",
            "Successful: 5 | Failed: 2\n",
            "Total Tokens: 300\n",
            "\n",
            "Top Models by Spend:\n",
            "  (no data)\n",
            "\n",
            "Top Providers by Spend:\n",
            "  (no data)"
        )
    );
}

#[test]
fn summary_usage_ranks_models_and_providers_by_spend() {
    let first = day(
        on(2026, 1, 1),
        breakdown(
            vec![("gpt-4o", entry("3", 2, 20)), ("claude", entry("5", 1, 10))],
            vec![("openai", entry("8", 3, 30))],
            Vec::new(),
        ),
    );
    let second = day(
        on(2026, 1, 2),
        breakdown(
            vec![("gpt-4o", entry("4", 1, 5))],
            vec![("anthropic", entry("1", 1, 10))],
            Vec::new(),
        ),
    );
    assert_eq!(
        summarise_usage_data(&page(vec![first, second], metadata("13", 3, 3, 0, 65))).unwrap(),
        concat!(
            "Total Spend: $13.0000\n",
            "Total Requests: 3\n",
            "Successful: 3 | Failed: 0\n",
            "Total Tokens: 65\n",
            "\n",
            "Top Models by Spend:\n",
            "  - gpt-4o: $7.0000 (3 reqs, 25 tokens)\n",
            "  - claude: $5.0000 (1 reqs, 10 tokens)\n",
            "\n",
            "Top Providers by Spend:\n",
            "  - openai: $8.0000 (3 reqs)\n",
            "  - anthropic: $1.0000 (1 reqs)"
        )
    );
}

#[test]
fn summary_usage_keeps_first_seen_order_on_a_spend_tie() {
    let first = day(
        on(2026, 1, 1),
        breakdown(vec![("zebra", entry("1", 1, 0))], Vec::new(), Vec::new()),
    );
    let second = day(
        on(2026, 1, 2),
        breakdown(vec![("alpha", entry("1", 1, 0))], Vec::new(), Vec::new()),
    );
    let text = summarise_usage_data(&page(vec![first, second], metadata("2", 2, 2, 0, 0))).unwrap();
    assert!(text.find("  - zebra:").unwrap() < text.find("  - alpha:").unwrap());
}

#[test]
fn summary_usage_cuts_the_lists_to_fifteen_and_ten() {
    let model_names: Vec<String> = (0..16_usize)
        .map(|index| format!("model-{index:02}"))
        .collect();
    let models: Vec<(&str, MetricWithMetadata)> = model_names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.as_str(), entry(&spend_of(index), 0, 0)))
        .collect();
    let provider_names: Vec<String> = (0..11_usize)
        .map(|index| format!("provider-{index:02}"))
        .collect();
    let providers: Vec<(&str, MetricWithMetadata)> = provider_names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.as_str(), entry(&spend_of(index), 0, 0)))
        .collect();
    let text = summarise_usage_data(&page(
        vec![day(
            on(2026, 1, 1),
            breakdown(models, providers, Vec::new()),
        )],
        metadata("0", 0, 0, 0, 0),
    ))
    .unwrap();
    assert_eq!(text.matches("  - model-").count(), 15);
    assert!(text.contains("  - model-14: $36.0000 (0 reqs, 0 tokens)"));
    assert!(!text.contains("model-15"));
    assert_eq!(text.matches("  - provider-").count(), 10);
    assert!(text.contains("  - provider-09: $41.0000 (0 reqs)"));
    assert!(!text.contains("provider-10"));
}

#[test]
fn summary_usage_rounds_money_to_four_places() {
    let text = summarise_usage_data(&page(
        vec![day(
            on(2026, 1, 1),
            breakdown(vec![("m", entry("1.23456", 1, 1))], Vec::new(), Vec::new()),
        )],
        metadata("1.23456", 1, 1, 0, 1),
    ))
    .unwrap();
    assert!(text.contains("Total Spend: $1.2346"));
    assert!(text.contains("  - m: $1.2346 (1 reqs, 1 tokens)"));
}

#[test]
fn summary_entity_empty_pages_use_the_no_data_sentence() {
    assert_eq!(
        summarise_entity_data(&empty_page(), EntityLabel::Team).unwrap(),
        "No Team usage data found for the given date range."
    );
    assert_eq!(
        summarise_entity_data(&empty_page(), EntityLabel::Tag).unwrap(),
        "No Tag usage data found for the given date range."
    );
}

#[test]
fn summary_entity_sums_days_ranks_and_keeps_the_first_alias() {
    let first = day(
        on(2026, 1, 1),
        breakdown(
            Vec::new(),
            Vec::new(),
            vec![
                ("team-1", alias_entry("Alpha", "2", 1, 10)),
                ("team-2", alias_entry("Beta", "9", 2, 20)),
            ],
        ),
    );
    let second = day(
        on(2026, 1, 2),
        breakdown(
            Vec::new(),
            Vec::new(),
            vec![("team-1", alias_entry("Renamed", "3", 1, 5))],
        ),
    );
    assert_eq!(
        summarise_entity_data(
            &page(vec![first, second], metadata("14", 4, 4, 0, 35)),
            EntityLabel::Team
        )
        .unwrap(),
        concat!(
            "Team Usage (2 teams):\n",
            "\n",
            "- Beta (ID: team-2): $9.0000 | 2 reqs | 20 tokens\n",
            "- Alpha (ID: team-1): $5.0000 | 2 reqs | 15 tokens"
        )
    );
}

#[test]
fn summary_entity_prints_the_plural_and_falls_back_to_the_id() {
    let text = summarise_entity_data(
        &page(
            vec![day(
                on(2026, 1, 1),
                breakdown(
                    Vec::new(),
                    Vec::new(),
                    vec![("tag-a", entry("0.12346", 1, 2))],
                ),
            )],
            metadata("0.12346", 1, 1, 0, 2),
        ),
        EntityLabel::Tag,
    )
    .unwrap();
    assert_eq!(
        text,
        "Tag Usage (1 tags):\n\n- tag-a (ID: tag-a): $0.1235 | 1 reqs | 2 tokens"
    );
}

#[test]
fn summary_entity_keeps_first_seen_order_on_a_spend_tie() {
    let text = summarise_entity_data(
        &page(
            vec![day(
                on(2026, 1, 1),
                breakdown(
                    Vec::new(),
                    Vec::new(),
                    vec![("second", entry("1", 1, 0)), ("first", entry("1", 1, 0))],
                ),
            )],
            metadata("2", 2, 2, 0, 0),
        ),
        EntityLabel::Team,
    )
    .unwrap();
    assert!(text.find("- second").unwrap() < text.find("- first").unwrap());
}

#[test]
fn summary_entity_rejects_a_non_text_alias() {
    let page = page(
        vec![day(
            on(2026, 1, 1),
            breakdown(
                Vec::new(),
                Vec::new(),
                vec![("team-1", non_text_alias_entry("1", 1, 1))],
            ),
        )],
        metadata("1", 1, 1, 0, 1),
    );
    assert_eq!(
        summarise_entity_data(&page, EntityLabel::Team)
            .unwrap_err()
            .to_string(),
        "the entity team-1 has an alias that is not text"
    );
}
