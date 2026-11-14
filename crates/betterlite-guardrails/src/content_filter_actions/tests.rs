#![cfg(test)]

mod content_filter_actions_behavior {
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};

    use betterlite_config::sections::content_category::loading::CategoryLoadResult;
    use betterlite_config::sections::content_category::model::{
        CategoryConfig, CompiledPhrasePattern,
    };
    use betterlite_config::sections::content_category::pattern::Pattern;
    use betterlite_config::sections::content_filter_records::category_types::ConditionalCategoryConfig;
    use betterlite_config::sections::content_filter_records::{
        BlockedWord, ContentFilterAction, ContentFilterDetection, ContentFilterPattern,
    };
    use reqwest::StatusCode;
    use rstest::rstest;

    use crate::content_filter_actions::error::{
        ContentFilterActionError, ContentFilterBlockDetail,
    };
    use crate::content_filter_actions::handlers::{
        handle_blocked_word_match, handle_category_keyword_match, handle_conditional_match,
        handle_pattern_match,
    };
    use crate::content_filter_actions::{ContentFilterSource, filter_single_text};
    use crate::content_filter_matching::{CategoryMatch, ContentFilterMatchingError};
    use crate::pattern_matching::CompiledPattern;
    use crate::pattern_matching::PatternError as ProcessPatternError;
    use crate::pattern_matching::compile;
    use crate::pattern_matching::masking::MaskError;
    use crate::primitives::pattern_matching::error::PatternError as ReplacementError;

    const KEYWORD_TAG: &str = "[KEYWORD_REDACTED]";
    const PATTERN_FORMAT: &str = "[{pattern_name}_REDACTED]";

    fn matched(
        phrase: &str,
        category: &str,
        severity: &str,
        action: ContentFilterAction,
    ) -> CategoryMatch {
        CategoryMatch {
            matched: phrase.to_owned(),
            category: category.to_owned(),
            severity: severity.to_owned(),
            action,
        }
    }

    fn regex_pattern(name: &str, source: &str, action: ContentFilterAction) -> CompiledPattern {
        compile(&ContentFilterPattern {
            pattern_type: "regex".to_owned(),
            pattern_name: None,
            pattern: Some(source.to_owned()),
            name: Some(name.to_owned()),
            action,
        })
        .expect("test regex pattern compiles")
    }

    fn blocked(
        keyword: &str,
        action: ContentFilterAction,
        description: Option<&str>,
    ) -> BlockedWord {
        BlockedWord {
            keyword: keyword.to_owned(),
            action,
            description: description.map(str::to_owned),
        }
    }

    fn category(name: &str, action: ContentFilterAction) -> CategoryConfig {
        CategoryConfig {
            category_name: name.to_owned(),
            description: String::new(),
            default_action: action,
            keywords: Vec::new(),
            exceptions: Vec::new(),
            identifier_words: Vec::new(),
            always_block_keywords: Vec::new(),
            inherit_from: None,
            additional_block_words: Vec::new(),
            phrase_patterns: Vec::new(),
        }
    }

    fn conditional(
        action: ContentFilterAction,
        severity: &str,
        identifiers: &[&str],
        blocks: &[&str],
    ) -> ConditionalCategoryConfig {
        ConditionalCategoryConfig {
            identifier_words: identifiers.iter().map(|word| (*word).to_owned()).collect(),
            block_words: blocks.iter().map(|word| (*word).to_owned()).collect(),
            action,
            severity: severity.to_owned(),
        }
    }

    fn phrase_category(name: &str, action: ContentFilterAction, source: &str) -> CategoryConfig {
        let mut config = category(name, action);
        config.phrase_patterns = vec![CompiledPhrasePattern {
            source: source.to_owned(),
            regex: Pattern::compile(source).expect("test phrase pattern compiles"),
        }];
        config
    }

    fn filter_source<'a>(
        categories: &'a CategoryLoadResult,
        patterns: &'a [CompiledPattern],
        blocked_words: &'a [BlockedWord],
    ) -> ContentFilterSource<'a> {
        ContentFilterSource {
            categories,
            patterns,
            blocked_words,
            pattern_redaction_format: PATTERN_FORMAT,
            keyword_redaction_tag: KEYWORD_TAG,
        }
    }

    fn keyword_categories(
        entries: &[(&str, &str, &str, ContentFilterAction)],
    ) -> CategoryLoadResult {
        let mut categories = CategoryLoadResult::default();
        for (keyword, category_name, severity, action) in entries {
            categories.category_keywords.insert(
                (*keyword).to_owned(),
                ((*category_name).to_owned(), (*severity).to_owned(), *action),
            );
        }
        categories
    }

    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);

    impl Write for Captured {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .map_err(|_poisoned| io::Error::other("poisoned log buffer"))?
                .extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn with_logs<Outcome, Run: FnOnce() -> Outcome>(run: Run) -> (Outcome, String) {
        let captured = Captured::default();
        let writer = captured.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::WARN)
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        let outcome = tracing::subscriber::with_default(subscriber, run);
        let buffer = captured.0.lock().expect("log buffer").clone();
        (
            outcome,
            String::from_utf8(buffer).expect("utf-8 log output"),
        )
    }

    #[test]
    fn conditional_match_appends_a_category_keyword_detection_before_the_action() {
        let matched = matched(
            "minor + romantic",
            "child_safety",
            "high",
            ContentFilterAction::Mask,
        );
        let mut detections = Vec::new();
        handle_conditional_match(&matched, Some(&mut detections)).unwrap();
        assert_eq!(detections.len(), 1);
        let ContentFilterDetection::CategoryKeyword(detection) = &detections[0] else {
            panic!("expected a category keyword detection");
        };
        assert_eq!(detection.r#type, "category_keyword");
        assert_eq!(detection.category, "child_safety");
        assert_eq!(detection.keyword, "minor + romantic");
        assert_eq!(detection.severity, "high");
        assert_eq!(detection.action, "MASK");
    }

    #[test]
    fn conditional_match_appends_the_detection_before_a_block_error() {
        let matched = matched(
            "minor + romantic",
            "child_safety",
            "high",
            ContentFilterAction::Block,
        );
        let mut detections = Vec::new();
        let error = handle_conditional_match(&matched, Some(&mut detections)).unwrap_err();
        assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
        assert_eq!(detections.len(), 1);
        let ContentFilterDetection::CategoryKeyword(detection) = &detections[0] else {
            panic!("expected a category keyword detection");
        };
        assert_eq!(detection.keyword, "minor + romantic");
        assert_eq!(detection.action, "BLOCK");
    }

    #[test]
    fn conditional_block_error_names_the_category_phrase_and_severity() {
        let matched = matched(
            "minor + romantic",
            "child_safety",
            "high",
            ContentFilterAction::Block,
        );
        let error = handle_conditional_match(&matched, None).unwrap_err();
        assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
        let ContentFilterActionError::Blocked { detail } = error else {
            panic!("expected a block error");
        };
        let ContentFilterBlockDetail::Conditional {
            error,
            category,
            matched_phrase,
            severity,
        } = detail
        else {
            panic!("expected the conditional detail");
        };
        assert_eq!(category, "child_safety");
        assert_eq!(matched_phrase, "minor + romantic");
        assert_eq!(severity, "high");
        assert!(error.contains("minor + romantic"));
    }

    #[test]
    fn only_a_block_is_a_bad_request() {
        let matching = ContentFilterActionError::Matching(ContentFilterMatchingError::Keyword(
            regex::Error::Syntax("invalid".to_owned()),
        ));
        assert_eq!(matching.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
        let redaction = ContentFilterActionError::Redaction(MaskError::TooLarge);
        assert_eq!(redaction.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
        let pattern = ContentFilterActionError::Pattern(ProcessPatternError::TooLarge);
        assert_eq!(pattern.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
        let replacement = ContentFilterActionError::Replacement(ReplacementError::Template);
        assert_eq!(replacement.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
        let keyword = ContentFilterActionError::Keyword(regex::Error::Syntax("invalid".to_owned()));
        assert_eq!(keyword.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn conditional_mask_logs_a_warning_and_leaves_the_text_unchanged() {
        let mut categories = CategoryLoadResult::default();
        categories.conditional_categories.insert(
            "child_safety".to_owned(),
            conditional(ContentFilterAction::Mask, "high", &["minor"], &["romantic"]),
        );
        let text = "a minor with romantic ideas";
        let mut detections = Vec::new();
        let (result, logs) = with_logs(|| {
            filter_single_text(
                &filter_source(&categories, &[], &[]),
                text,
                Some(&mut detections),
            )
        });
        assert_eq!(result.unwrap(), text);
        assert!(logs.contains("MASK action is not supported for conditional categories"));
        assert_eq!(detections.len(), 1);
    }

    #[test]
    fn phrase_pattern_mask_is_reported_without_redaction() {
        let mut categories = CategoryLoadResult::default();
        categories.loaded_categories.insert(
            "financial_advice".to_owned(),
            phrase_category(
                "financial_advice",
                ContentFilterAction::Mask,
                "put my money to make it grow",
            ),
        );
        let text = "please put my money to make it grow";
        let mut detections = Vec::new();
        let (result, logs) = with_logs(|| {
            filter_single_text(
                &filter_source(&categories, &[], &[]),
                text,
                Some(&mut detections),
            )
        });
        assert_eq!(result.unwrap(), text);
        assert!(logs.contains("MASK action is not supported for conditional categories"));
        let ContentFilterDetection::CategoryKeyword(detection) = &detections[0] else {
            panic!("expected a category keyword detection");
        };
        assert_eq!(detection.keyword, "phrase: put my money to make it grow");
        assert_eq!(detection.severity, "high");
        assert_eq!(detection.action, "MASK");
    }

    #[test]
    fn category_keyword_appends_a_detection_before_the_action() {
        let matched = matched("alert", "operational", "medium", ContentFilterAction::Mask);
        let mut detections = Vec::new();
        let masked =
            handle_category_keyword_match(&matched, "an ALERT", KEYWORD_TAG, Some(&mut detections))
                .unwrap();
        assert_eq!(masked, "an [KEYWORD_REDACTED]");
        assert_eq!(detections.len(), 1);
        let ContentFilterDetection::CategoryKeyword(detection) = &detections[0] else {
            panic!("expected a category keyword detection");
        };
        assert_eq!(detection.r#type, "category_keyword");
        assert_eq!(detection.category, "operational");
        assert_eq!(detection.keyword, "alert");
        assert_eq!(detection.severity, "medium");
        assert_eq!(detection.action, "MASK");
    }

    #[test]
    fn category_keyword_appends_the_detection_before_a_block_error() {
        let matched = matched("alert", "operational", "medium", ContentFilterAction::Block);
        let mut detections = Vec::new();
        let error =
            handle_category_keyword_match(&matched, "an alert", KEYWORD_TAG, Some(&mut detections))
                .unwrap_err();
        assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
        assert_eq!(detections.len(), 1);
        let ContentFilterDetection::CategoryKeyword(detection) = &detections[0] else {
            panic!("expected a category keyword detection");
        };
        assert_eq!(detection.keyword, "alert");
        assert_eq!(detection.action, "BLOCK");
    }

    #[test]
    fn category_keyword_block_error_names_the_keyword_category_and_severity() {
        let matched = matched("alert", "operational", "medium", ContentFilterAction::Block);
        let error =
            handle_category_keyword_match(&matched, "an alert", KEYWORD_TAG, None).unwrap_err();
        assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
        let ContentFilterActionError::Blocked { detail } = error else {
            panic!("expected a block error");
        };
        let ContentFilterBlockDetail::CategoryKeyword {
            error,
            category,
            keyword,
            severity,
        } = detail
        else {
            panic!("expected the category keyword detail");
        };
        assert_eq!(category, "operational");
        assert_eq!(keyword, "alert");
        assert_eq!(severity, "medium");
        assert!(error.contains("alert"));
    }

    #[rstest]
    #[case::every_occurrence(
        "alert",
        "ALERT and alert",
        "[KEYWORD_REDACTED] and [KEYWORD_REDACTED]"
    )]
    #[case::inside_a_longer_word("men", "recommend", "recom[KEYWORD_REDACTED]d")]
    #[case::asterisk_wildcard("fu*c*k", "FUCK", "[KEYWORD_REDACTED]")]
    #[case::one_character_for_a_star("fu*c*k", "FUUCK", "[KEYWORD_REDACTED]")]
    fn category_keyword_mask_redacts_case_insensitively(
        #[case] keyword: &str,
        #[case] text: &str,
        #[case] expected: &str,
    ) {
        let matched = matched(keyword, "category", "high", ContentFilterAction::Mask);
        let masked = handle_category_keyword_match(&matched, text, KEYWORD_TAG, None).unwrap();
        assert_eq!(masked, expected);
    }

    #[test]
    fn pattern_match_appends_a_pattern_detection_before_the_action() {
        let pattern = regex_pattern("email", "a@b", ContentFilterAction::Mask);
        let spans = pattern.find_spans("a@b").unwrap();
        let mut detections = Vec::new();
        let masked = handle_pattern_match(
            &pattern,
            "a@b",
            &spans,
            PATTERN_FORMAT,
            Some(&mut detections),
        )
        .unwrap();
        assert_eq!(masked, "[EMAIL_REDACTED]");
        assert_eq!(detections.len(), 1);
        let ContentFilterDetection::Pattern(detection) = &detections[0] else {
            panic!("expected a pattern detection");
        };
        assert_eq!(detection.r#type, "pattern");
        assert_eq!(detection.pattern_name, "email");
        assert_eq!(detection.action, "MASK");
    }

    #[test]
    fn pattern_match_appends_the_detection_before_a_block_error() {
        let pattern = regex_pattern("email", "a@b", ContentFilterAction::Block);
        let spans = pattern.find_spans("a@b").unwrap();
        let mut detections = Vec::new();
        let error = handle_pattern_match(
            &pattern,
            "a@b",
            &spans,
            PATTERN_FORMAT,
            Some(&mut detections),
        )
        .unwrap_err();
        assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
        assert_eq!(detections.len(), 1);
        let ContentFilterDetection::Pattern(detection) = &detections[0] else {
            panic!("expected a pattern detection");
        };
        assert_eq!(detection.pattern_name, "email");
        assert_eq!(detection.action, "BLOCK");
    }

    #[test]
    fn pattern_block_error_names_the_pattern() {
        let pattern = regex_pattern("email", "a@b", ContentFilterAction::Block);
        let spans = pattern.find_spans("a@b").unwrap();
        let error =
            handle_pattern_match(&pattern, "a@b", &spans, PATTERN_FORMAT, None).unwrap_err();
        assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
        let ContentFilterActionError::Blocked { detail } = error else {
            panic!("expected a block error");
        };
        let ContentFilterBlockDetail::Pattern { error, pattern } = detail else {
            panic!("expected the pattern detail");
        };
        assert_eq!(pattern, "email");
        assert!(error.contains("email"));
    }

    #[test]
    fn pattern_mask_uses_the_uppercased_name_for_every_span() {
        let pattern = regex_pattern("email", r"\w+@\w+", ContentFilterAction::Mask);
        let text = "a@b and c@d";
        let spans = pattern.find_spans(text).unwrap();
        assert_eq!(spans.len(), 2);
        let masked = handle_pattern_match(&pattern, text, &spans, PATTERN_FORMAT, None).unwrap();
        assert_eq!(masked, "[EMAIL_REDACTED] and [EMAIL_REDACTED]");
    }

    #[test]
    fn blocked_word_appends_a_detection_before_the_action() {
        let word = blocked("hunter2", ContentFilterAction::Mask, Some("password"));
        let mut detections = Vec::new();
        let masked =
            handle_blocked_word_match(&word, "HUNTER2", KEYWORD_TAG, Some(&mut detections))
                .unwrap();
        assert_eq!(masked, KEYWORD_TAG);
        assert_eq!(detections.len(), 1);
        let ContentFilterDetection::BlockedWord(detection) = &detections[0] else {
            panic!("expected a blocked word detection");
        };
        assert_eq!(detection.r#type, "blocked_word");
        assert_eq!(detection.keyword, "hunter2");
        assert_eq!(detection.action, "MASK");
        assert_eq!(detection.description.as_deref(), Some("password"));
    }

    #[test]
    fn blocked_word_appends_the_detection_before_a_block_error() {
        let word = blocked("hunter2", ContentFilterAction::Block, None);
        let mut detections = Vec::new();
        let error = handle_blocked_word_match(&word, "hunter2", KEYWORD_TAG, Some(&mut detections))
            .unwrap_err();
        assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
        assert_eq!(detections.len(), 1);
        let ContentFilterDetection::BlockedWord(detection) = &detections[0] else {
            panic!("expected a blocked word detection");
        };
        assert_eq!(detection.keyword, "hunter2");
        assert_eq!(detection.action, "BLOCK");
    }

    #[rstest]
    #[case::with_description(
        Some("do not share it"),
        Some("do not share it"),
        "content blocked: keyword 'hunter2' detected (do not share it)"
    )]
    #[case::without_description(None, None, "content blocked: keyword 'hunter2' detected")]
    fn blocked_word_block_error_reports_the_optional_description(
        #[case] description: Option<&str>,
        #[case] expected_description: Option<&str>,
        #[case] expected_error: &str,
    ) {
        let word = blocked("hunter2", ContentFilterAction::Block, description);
        let error = handle_blocked_word_match(&word, "hunter2", KEYWORD_TAG, None).unwrap_err();
        assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
        let ContentFilterActionError::Blocked { detail } = error else {
            panic!("expected a block error");
        };
        let ContentFilterBlockDetail::BlockedWord {
            error,
            keyword,
            description: detail_description,
        } = detail
        else {
            panic!("expected the blocked word detail");
        };
        assert_eq!(error, expected_error);
        assert_eq!(keyword, "hunter2");
        assert_eq!(detail_description.as_deref(), expected_description);
    }

    #[rstest]
    #[case::upper_case("SECRET", "[KEYWORD_REDACTED]")]
    #[case::inside_a_longer_word("secrets", "[KEYWORD_REDACTED]s")]
    fn blocked_word_mask_redacts_case_insensitively(#[case] text: &str, #[case] expected: &str) {
        let word = blocked("secret", ContentFilterAction::Mask, None);
        assert_eq!(
            handle_blocked_word_match(&word, text, KEYWORD_TAG, None).unwrap(),
            expected
        );
    }

    #[test]
    fn handlers_accept_a_missing_detection_list() {
        let conditional_match = matched(
            "minor + romantic",
            "child_safety",
            "high",
            ContentFilterAction::Mask,
        );
        assert!(handle_conditional_match(&conditional_match, None).is_ok());
        let keyword = matched("alert", "operational", "medium", ContentFilterAction::Mask);
        assert_eq!(
            handle_category_keyword_match(&keyword, "alert", KEYWORD_TAG, None).unwrap(),
            KEYWORD_TAG
        );
        let word = blocked("secret", ContentFilterAction::Mask, None);
        assert_eq!(
            handle_blocked_word_match(&word, "SECRET", KEYWORD_TAG, None).unwrap(),
            KEYWORD_TAG
        );
        let pattern = regex_pattern("email", "a@b", ContentFilterAction::Mask);
        let spans = pattern.find_spans("a@b").unwrap();
        assert_eq!(
            handle_pattern_match(&pattern, "a@b", &spans, PATTERN_FORMAT, None).unwrap(),
            "[EMAIL_REDACTED]"
        );
        let blocking = matched(
            "minor + romantic",
            "child_safety",
            "high",
            ContentFilterAction::Block,
        );
        assert_eq!(
            handle_conditional_match(&blocking, None)
                .unwrap_err()
                .status_code(),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn filter_collects_exceptions_from_every_loaded_category() {
        let mut exempt = category("alpha", ContentFilterAction::Mask);
        exempt.exceptions.push("safe".to_owned());
        let mut flagged = category("beta", ContentFilterAction::Mask);
        flagged.exceptions.push("ok".to_owned());
        let mut categories = CategoryLoadResult::default();
        categories
            .loaded_categories
            .insert("alpha".to_owned(), exempt);
        categories
            .loaded_categories
            .insert("beta".to_owned(), flagged);
        categories.category_keywords.insert(
            "danger".to_owned(),
            (
                "beta".to_owned(),
                "high".to_owned(),
                ContentFilterAction::Mask,
            ),
        );
        let source = filter_source(&categories, &[], &[]);
        let mut detections = Vec::new();
        assert_eq!(
            filter_single_text(&source, "danger safe", Some(&mut detections)).unwrap(),
            "danger safe"
        );
        assert!(detections.is_empty());
        assert_eq!(
            filter_single_text(&source, "danger ok", Some(&mut detections)).unwrap(),
            "danger ok"
        );
        assert!(detections.is_empty());
        assert_eq!(
            filter_single_text(&source, "danger", Some(&mut detections)).unwrap(),
            KEYWORD_TAG
        );
        assert_eq!(detections.len(), 1);
    }

    fn full_source() -> (CategoryLoadResult, Vec<CompiledPattern>, Vec<BlockedWord>) {
        let mut categories = CategoryLoadResult::default();
        categories.loaded_categories.insert(
            "child_safety".to_owned(),
            phrase_category(
                "child_safety",
                ContentFilterAction::Mask,
                "run away from home",
            ),
        );
        categories.conditional_categories.insert(
            "child_safety".to_owned(),
            conditional(ContentFilterAction::Mask, "high", &["minor"], &["romantic"]),
        );
        categories.category_keywords.insert(
            "danger".to_owned(),
            (
                "risk".to_owned(),
                "high".to_owned(),
                ContentFilterAction::Mask,
            ),
        );
        let patterns = vec![
            regex_pattern("secret", "secret", ContentFilterAction::Mask),
            regex_pattern("token", "token", ContentFilterAction::Mask),
        ];
        let words = vec![blocked("hunter2", ContentFilterAction::Mask, None)];
        (categories, patterns, words)
    }

    fn detection_kinds(detections: &[ContentFilterDetection]) -> Vec<(&str, &str, &str)> {
        detections
            .iter()
            .map(|detection| match detection {
                ContentFilterDetection::CategoryKeyword(detection) => (
                    "category_keyword",
                    detection.keyword.as_str(),
                    detection.action.as_str(),
                ),
                ContentFilterDetection::Pattern(detection) => (
                    "pattern",
                    detection.pattern_name.as_str(),
                    detection.action.as_str(),
                ),
                ContentFilterDetection::BlockedWord(detection) => (
                    "blocked_word",
                    detection.keyword.as_str(),
                    detection.action.as_str(),
                ),
                ContentFilterDetection::CompetitorIntent(_) => {
                    panic!("unexpected competitor intent detection")
                }
            })
            .collect()
    }

    #[test]
    fn filter_checks_conditional_phrase_keyword_pattern_then_blocked_word() {
        let (categories, patterns, words) = full_source();
        let text = "a minor with romantic ideas wants to run away from home and share the secret token danger hunter2";
        let mut detections = Vec::new();
        let filtered = filter_single_text(
            &filter_source(&categories, &patterns, &words),
            text,
            Some(&mut detections),
        )
        .unwrap();
        assert_eq!(
            filtered,
            "a minor with romantic ideas wants to run away from home and share the [SECRET_REDACTED] [TOKEN_REDACTED] [KEYWORD_REDACTED] [KEYWORD_REDACTED]"
        );
        assert_eq!(
            detection_kinds(&detections),
            vec![
                ("category_keyword", "minor + romantic", "MASK"),
                ("category_keyword", "phrase: run away from home", "MASK"),
                ("category_keyword", "danger", "MASK"),
                ("pattern", "secret", "MASK"),
                ("pattern", "token", "MASK"),
                ("blocked_word", "hunter2", "MASK"),
            ]
        );
    }

    #[test]
    fn filter_updates_the_lowercased_text_after_each_masked_blocked_word() {
        let categories = CategoryLoadResult::default();
        let words = vec![
            blocked("foo", ContentFilterAction::Mask, None),
            blocked("redacted", ContentFilterAction::Mask, None),
        ];
        let source = ContentFilterSource {
            categories: &categories,
            patterns: &[],
            blocked_words: &words,
            pattern_redaction_format: PATTERN_FORMAT,
            keyword_redaction_tag: "[REDACTED]",
        };
        let mut detections = Vec::new();
        let filtered = filter_single_text(&source, "foo", Some(&mut detections)).unwrap();
        assert_eq!(filtered, "[[REDACTED]]");
        assert_eq!(
            detection_kinds(&detections),
            vec![
                ("blocked_word", "foo", "MASK"),
                ("blocked_word", "redacted", "MASK"),
            ]
        );
    }

    #[test]
    fn filter_stops_after_a_blocking_conditional_match() {
        let mut categories = CategoryLoadResult::default();
        categories.conditional_categories.insert(
            "child_safety".to_owned(),
            conditional(
                ContentFilterAction::Block,
                "high",
                &["minor"],
                &["romantic"],
            ),
        );
        let patterns = vec![regex_pattern(
            "romantic",
            "romantic",
            ContentFilterAction::Mask,
        )];
        let words = vec![blocked("romantic", ContentFilterAction::Mask, None)];
        let mut detections = Vec::new();
        let error = filter_single_text(
            &filter_source(&categories, &patterns, &words),
            "a minor with romantic ideas",
            Some(&mut detections),
        )
        .unwrap_err();
        assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
        assert_eq!(detections.len(), 1);
        let ContentFilterDetection::CategoryKeyword(detection) = &detections[0] else {
            panic!("expected a category keyword detection");
        };
        assert_eq!(detection.keyword, "minor + romantic");
        assert_eq!(detection.action, "BLOCK");
    }

    #[test]
    fn filter_masks_category_keywords_before_it_searches_patterns() {
        let categories =
            keyword_categories(&[("danger", "risk", "high", ContentFilterAction::Mask)]);
        let patterns = vec![regex_pattern("leak", "danger", ContentFilterAction::Mask)];
        let mut detections = Vec::new();
        let filtered = filter_single_text(
            &filter_source(&categories, &patterns, &[]),
            "danger",
            Some(&mut detections),
        )
        .unwrap();
        assert_eq!(filtered, KEYWORD_TAG);
        assert_eq!(
            detection_kinds(&detections),
            vec![("category_keyword", "danger", "MASK")]
        );
    }

    #[test]
    fn filter_returns_the_text_when_nothing_matches() {
        let categories = CategoryLoadResult::default();
        let words = vec![blocked("absent", ContentFilterAction::Mask, None)];
        let mut detections = Vec::new();
        assert_eq!(
            filter_single_text(
                &filter_source(&categories, &[], &words),
                "plain text",
                Some(&mut detections),
            )
            .unwrap(),
            "plain text"
        );
        assert!(detections.is_empty());
    }

    #[test]
    fn filter_masks_without_a_detection_list() {
        let categories =
            keyword_categories(&[("danger", "risk", "high", ContentFilterAction::Mask)]);
        assert_eq!(
            filter_single_text(&filter_source(&categories, &[], &[]), "danger", None).unwrap(),
            KEYWORD_TAG
        );
    }
}
