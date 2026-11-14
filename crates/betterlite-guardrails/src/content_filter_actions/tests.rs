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
    use crate::content_filter_actions::handlers::blocked_word::handle_blocked_word_match;
    use crate::content_filter_actions::handlers::category_keyword::handle_category_keyword_match;
    use crate::content_filter_actions::handlers::conditional::handle_conditional_match;
    use crate::content_filter_actions::handlers::pattern::handle_pattern_match;
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
        let identifier_words = identifiers.iter().map(|word| (*word).to_owned()).collect();
        let block_words = blocks.iter().map(|word| (*word).to_owned()).collect();
        ConditionalCategoryConfig {
            identifier_words,
            block_words,
            action,
            severity: severity.to_owned(),
        }
    }

    fn phrase(source: &str) -> CompiledPhrasePattern {
        CompiledPhrasePattern {
            source: source.to_owned(),
            regex: Pattern::compile(source).expect("test phrase pattern compiles"),
        }
    }

    fn phrase_category(name: &str, action: ContentFilterAction, source: &str) -> CategoryConfig {
        let mut config = category(name, action);
        config.phrase_patterns.push(phrase(source));
        config
    }

    fn filter_source<'source>(
        categories: &'source CategoryLoadResult,
        patterns: &'source [CompiledPattern],
        blocked_words: &'source [BlockedWord],
    ) -> ContentFilterSource<'source> {
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
        with_logs_at(tracing::Level::WARN, run)
    }

    fn with_logs_at<Outcome, Run: FnOnce() -> Outcome>(
        level: tracing::Level,
        run: Run,
    ) -> (Outcome, String) {
        let captured = Captured::default();
        let writer = captured.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(level)
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
        let returned = handle_conditional_match(
            &matched,
            "a minor with romantic ideas",
            Some(&mut detections),
        )
        .unwrap();
        assert_eq!(returned, "a minor with romantic ideas");
        assert_eq!(detections.len(), 1);
        let ContentFilterDetection::CategoryKeyword(record) = &detections[0] else {
            panic!("expected a category keyword record");
        };
        assert_eq!(record.r#type, "category_keyword");
        assert_eq!(record.category, "child_safety");
        assert_eq!(record.keyword, "minor + romantic");
        assert_eq!(record.severity, "high");
        assert_eq!(record.action, "MASK");
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
        let error = handle_conditional_match(
            &matched,
            "a minor with romantic ideas",
            Some(&mut detections),
        )
        .unwrap_err();
        assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
        assert_eq!(detections.len(), 1);
        let ContentFilterDetection::CategoryKeyword(record) = &detections[0] else {
            panic!("expected a category keyword record");
        };
        assert_eq!(record.keyword, "minor + romantic");
        assert_eq!(record.action, "BLOCK");
    }

    #[test]
    fn conditional_block_error_names_the_category_phrase_and_severity() {
        let matched = matched(
            "minor + romantic",
            "child_safety",
            "high",
            ContentFilterAction::Block,
        );
        let failure =
            handle_conditional_match(&matched, "a minor with romantic ideas", None).unwrap_err();
        assert_eq!(failure.status_code(), StatusCode::BAD_REQUEST);
        let ContentFilterActionError::Blocked { detail } = failure else {
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
        assert_eq!(
            error,
            "Content blocked: child_safety conditional match 'minor + romantic' detected (severity: high)"
        );
    }

    #[test]
    fn blocked_match_logs_the_block_message() {
        let pattern = regex_pattern("email", "a@b", ContentFilterAction::Block);
        let spans = pattern.find_spans("a@b").unwrap();
        let (outcome, logs) =
            with_logs(|| handle_pattern_match(&pattern, "a@b", &spans, PATTERN_FORMAT, None));
        assert_eq!(outcome.unwrap_err().status_code(), StatusCode::BAD_REQUEST);
        assert!(logs.contains("Content blocked: email pattern detected"));
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
    fn block_details_serialize_conditional_and_keyword_objects() {
        let conditional = ContentFilterBlockDetail::Conditional {
            error: "Content blocked: child_safety conditional match 'minor + romantic' detected (severity: high)"
                .to_owned(),
            category: "child_safety".to_owned(),
            matched_phrase: "minor + romantic".to_owned(),
            severity: "high".to_owned(),
        };
        assert_eq!(
            serde_json::to_string(&conditional).unwrap(),
            r#"{"error":"Content blocked: child_safety conditional match 'minor + romantic' detected (severity: high)","category":"child_safety","matched_phrase":"minor + romantic","severity":"high"}"#
        );
        let keyword = ContentFilterBlockDetail::CategoryKeyword {
            error:
                "Content blocked: operational category keyword 'alert' detected (severity: medium)"
                    .to_owned(),
            category: "operational".to_owned(),
            keyword: "alert".to_owned(),
            severity: "medium".to_owned(),
        };
        assert_eq!(
            serde_json::to_string(&keyword).unwrap(),
            r#"{"error":"Content blocked: operational category keyword 'alert' detected (severity: medium)","category":"operational","keyword":"alert","severity":"medium"}"#
        );
    }

    #[test]
    fn block_details_serialize_pattern_and_blocked_word_objects() {
        let pattern = ContentFilterBlockDetail::Pattern {
            error: "Content blocked: email pattern detected".to_owned(),
            pattern: "email".to_owned(),
        };
        assert_eq!(
            serde_json::to_string(&pattern).unwrap(),
            r#"{"error":"Content blocked: email pattern detected","pattern":"email"}"#
        );
        let word = ContentFilterBlockDetail::BlockedWord {
            error: "Content blocked: keyword 'hunter2' detected".to_owned(),
            keyword: "hunter2".to_owned(),
            description: None,
        };
        assert_eq!(
            serde_json::to_string(&word).unwrap(),
            r#"{"error":"Content blocked: keyword 'hunter2' detected","keyword":"hunter2","description":null}"#
        );
        let described = ContentFilterBlockDetail::BlockedWord {
            error: "Content blocked: keyword 'hunter2' detected (do not share it)".to_owned(),
            keyword: "hunter2".to_owned(),
            description: Some("do not share it".to_owned()),
        };
        assert_eq!(
            serde_json::to_string(&described).unwrap(),
            r#"{"error":"Content blocked: keyword 'hunter2' detected (do not share it)","keyword":"hunter2","description":"do not share it"}"#
        );
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
        let ContentFilterDetection::CategoryKeyword(record) = &detections[0] else {
            panic!("expected a category keyword record");
        };
        assert_eq!(record.keyword, "phrase: put my money to make it grow");
        assert_eq!(record.severity, "high");
        assert_eq!(record.action, "MASK");
    }

    #[test]
    fn filter_blocks_on_a_phrase_pattern() {
        let mut categories = CategoryLoadResult::default();
        categories.loaded_categories.insert(
            "financial_advice".to_owned(),
            phrase_category(
                "financial_advice",
                ContentFilterAction::Block,
                "put my money to make it grow",
            ),
        );
        let mut detections = Vec::new();
        let error = filter_single_text(
            &filter_source(&categories, &[], &[]),
            "please put my money to make it grow",
            Some(&mut detections),
        )
        .unwrap_err();
        assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
        let ContentFilterDetection::CategoryKeyword(record) = &detections[0] else {
            panic!("expected a category keyword record");
        };
        assert_eq!(record.keyword, "phrase: put my money to make it grow");
        assert_eq!(record.action, "BLOCK");
    }

    #[test]
    fn category_keyword_appends_a_detection_before_the_action() {
        let matched = matched("alert", "operational", "medium", ContentFilterAction::Mask);
        let mut detections = Vec::new();
        let (outcome, logs) = with_logs_at(tracing::Level::INFO, || {
            handle_category_keyword_match(&matched, "an ALERT", KEYWORD_TAG, Some(&mut detections))
        });
        assert!(logs.contains("masked category keyword"));
        assert_eq!(outcome.unwrap(), "an [KEYWORD_REDACTED]");
        assert_eq!(detections.len(), 1);
        let ContentFilterDetection::CategoryKeyword(record) = &detections[0] else {
            panic!("expected a category keyword record");
        };
        assert_eq!(record.r#type, "category_keyword");
        assert_eq!(record.category, "operational");
        assert_eq!(record.keyword, "alert");
        assert_eq!(record.severity, "medium");
        assert_eq!(record.action, "MASK");
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
        let ContentFilterDetection::CategoryKeyword(record) = &detections[0] else {
            panic!("expected a category keyword record");
        };
        assert_eq!(record.keyword, "alert");
        assert_eq!(record.action, "BLOCK");
    }

    #[test]
    fn category_keyword_block_error_names_the_keyword_category_and_severity() {
        let matched = matched("alert", "operational", "medium", ContentFilterAction::Block);
        let failure =
            handle_category_keyword_match(&matched, "an alert", KEYWORD_TAG, None).unwrap_err();
        assert_eq!(failure.status_code(), StatusCode::BAD_REQUEST);
        let ContentFilterActionError::Blocked { detail } = failure else {
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
        assert_eq!(
            error,
            "Content blocked: operational category keyword 'alert' detected (severity: medium)"
        );
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
        let (outcome, logs) = with_logs_at(tracing::Level::INFO, || {
            handle_pattern_match(
                &pattern,
                "a@b",
                &spans,
                PATTERN_FORMAT,
                Some(&mut detections),
            )
        });
        assert!(logs.contains("masked every pattern match"));
        assert_eq!(outcome.unwrap(), "[EMAIL_REDACTED]");
        assert_eq!(detections.len(), 1);
        let ContentFilterDetection::Pattern(record) = &detections[0] else {
            panic!("expected a pattern record");
        };
        assert_eq!(record.r#type, "pattern");
        assert_eq!(record.pattern_name, "email");
        assert_eq!(record.action, "MASK");
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
        let ContentFilterDetection::Pattern(record) = &detections[0] else {
            panic!("expected a pattern record");
        };
        assert_eq!(record.pattern_name, "email");
        assert_eq!(record.action, "BLOCK");
    }

    #[test]
    fn pattern_block_error_names_the_pattern() {
        let pattern = regex_pattern("email", "a@b", ContentFilterAction::Block);
        let spans = pattern.find_spans("a@b").unwrap();
        let failure =
            handle_pattern_match(&pattern, "a@b", &spans, PATTERN_FORMAT, None).unwrap_err();
        assert_eq!(failure.status_code(), StatusCode::BAD_REQUEST);
        let ContentFilterActionError::Blocked { detail } = failure else {
            panic!("expected a block error");
        };
        let ContentFilterBlockDetail::Pattern {
            error,
            pattern: name,
        } = detail
        else {
            panic!("expected the pattern detail");
        };
        assert_eq!(name, "email");
        assert_eq!(error, "Content blocked: email pattern detected");
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
        let ContentFilterDetection::BlockedWord(record) = &detections[0] else {
            panic!("expected a blocked word record");
        };
        assert_eq!(record.r#type, "blocked_word");
        assert_eq!(record.keyword, "hunter2");
        assert_eq!(record.action, "MASK");
        assert_eq!(record.description.as_deref(), Some("password"));
    }

    #[test]
    fn blocked_word_match_logs_the_keyword_and_action_at_debug() {
        let word = blocked("hunter2", ContentFilterAction::Mask, None);
        let (outcome, logs) = with_logs_at(tracing::Level::DEBUG, || {
            handle_blocked_word_match(&word, "hunter2", KEYWORD_TAG, None)
        });
        assert_eq!(outcome.unwrap(), KEYWORD_TAG);
        assert!(logs.contains("blocked word found"));
        assert!(logs.contains("keyword=hunter2"));
        assert!(logs.contains("action=MASK"));
        assert!(logs.contains("masked blocked word"));
    }

    #[test]
    fn blocked_word_appends_the_detection_before_a_block_error() {
        let word = blocked("hunter2", ContentFilterAction::Block, None);
        let mut detections = Vec::new();
        let error = handle_blocked_word_match(&word, "hunter2", KEYWORD_TAG, Some(&mut detections))
            .unwrap_err();
        assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
        assert_eq!(detections.len(), 1);
        let ContentFilterDetection::BlockedWord(record) = &detections[0] else {
            panic!("expected a blocked word record");
        };
        assert_eq!(record.keyword, "hunter2");
        assert_eq!(record.action, "BLOCK");
    }

    #[rstest]
    #[case::with_description(
        Some("do not share it"),
        Some("do not share it"),
        "Content blocked: keyword 'hunter2' detected (do not share it)"
    )]
    #[case::without_description(None, None, "Content blocked: keyword 'hunter2' detected")]
    fn blocked_word_block_error_reports_the_optional_description(
        #[case] description: Option<&str>,
        #[case] expected_description: Option<&str>,
        #[case] expected_error: &str,
    ) {
        let word = blocked("hunter2", ContentFilterAction::Block, description);
        let failure = handle_blocked_word_match(&word, "hunter2", KEYWORD_TAG, None).unwrap_err();
        assert_eq!(failure.status_code(), StatusCode::BAD_REQUEST);
        let ContentFilterActionError::Blocked { detail } = failure else {
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
        assert_eq!(
            handle_conditional_match(&conditional_match, "a minor with romantic ideas", None)
                .unwrap(),
            "a minor with romantic ideas"
        );
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
            handle_conditional_match(&blocking, "a minor with romantic ideas", None)
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
            .map(|entry| match entry {
                ContentFilterDetection::CategoryKeyword(record) => (
                    "category_keyword",
                    record.keyword.as_str(),
                    record.action.as_str(),
                ),
                ContentFilterDetection::Pattern(record) => (
                    "pattern",
                    record.pattern_name.as_str(),
                    record.action.as_str(),
                ),
                ContentFilterDetection::BlockedWord(record) => (
                    "blocked_word",
                    record.keyword.as_str(),
                    record.action.as_str(),
                ),
                ContentFilterDetection::CompetitorIntent(_) => {
                    panic!("unexpected competitor intent record")
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
        let words = [
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
    fn filter_searches_blocked_words_in_the_pattern_masked_text() {
        let categories = CategoryLoadResult::default();
        let patterns = [regex_pattern("leak", "secret", ContentFilterAction::Mask)];
        let words = [blocked("redacted", ContentFilterAction::Mask, None)];
        let mut detections = Vec::new();
        let filtered = filter_single_text(
            &filter_source(&categories, &patterns, &words),
            "a secret",
            Some(&mut detections),
        )
        .unwrap();
        assert_eq!(filtered, "a [LEAK_[KEYWORD_REDACTED]]");
        assert_eq!(
            detection_kinds(&detections),
            vec![
                ("pattern", "leak", "MASK"),
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
        let patterns = [regex_pattern(
            "romantic",
            "romantic",
            ContentFilterAction::Mask,
        )];
        let words = [blocked("romantic", ContentFilterAction::Mask, None)];
        let mut detections = Vec::new();
        let error = filter_single_text(
            &filter_source(&categories, &patterns, &words),
            "a minor with romantic ideas",
            Some(&mut detections),
        )
        .unwrap_err();
        assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
        assert_eq!(detections.len(), 1);
        let ContentFilterDetection::CategoryKeyword(record) = &detections[0] else {
            panic!("expected a category keyword record");
        };
        assert_eq!(record.keyword, "minor + romantic");
        assert_eq!(record.action, "BLOCK");
    }

    #[test]
    fn filter_masks_category_keywords_before_it_searches_patterns() {
        let categories =
            keyword_categories(&[("danger", "risk", "high", ContentFilterAction::Mask)]);
        let patterns = [regex_pattern("leak", "danger", ContentFilterAction::Mask)];
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
        let patterns = [regex_pattern("leak", "danger", ContentFilterAction::Block)];
        let words = [blocked("absent", ContentFilterAction::Block, None)];
        let mut detections = Vec::new();
        assert_eq!(
            filter_single_text(
                &filter_source(&categories, &patterns, &words),
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
