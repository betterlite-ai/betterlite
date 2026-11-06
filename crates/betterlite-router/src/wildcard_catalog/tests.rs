#![cfg(test)]

use std::error::Error;

use regex::Regex;
use rstest::rstest;

use super::error::PatternCatalogError;
use super::{Candidate, PatternCatalog};

type Deployment = (&'static str, &'static str, &'static str);
type Resolved = (String, String);
type Expectation = Option<Vec<(&'static str, &'static str)>>;

enum Lookup {
    Route(&'static str),
    Filtered(&'static str, &'static [&'static str]),
    GetPattern(&'static str, Option<&'static str>),
    Deployments(&'static str, Option<&'static str>),
}

enum Boundary {
    Routes {
        deployments: Vec<Deployment>,
        removed: Vec<&'static str>,
        lookups: Vec<Lookup>,
        expectations: Vec<Expectation>,
    },
    UnidentifiedCandidate,
    RejectedCompile,
}

fn candidate(id: &str, model: &str) -> Candidate {
    Candidate {
        model_id: Some(id.to_owned()),
        model: model.to_owned(),
    }
}

fn catalog(deployments: &[Deployment], removed: &[&str]) -> PatternCatalog {
    let mut catalog = PatternCatalog::default();
    for (pattern, id, model) in deployments {
        catalog.add_pattern(pattern, candidate(id, model)).unwrap();
    }
    for id in removed {
        catalog.remove_deployment(id);
    }
    catalog
}

fn resolve(catalog: &PatternCatalog, lookup: &Lookup) -> Option<Vec<Resolved>> {
    let matched = match lookup {
        Lookup::Route(request) => catalog.route(request, None),
        Lookup::Filtered(request, patterns) => {
            let keys: Vec<String> = patterns.iter().map(|name| (*name).to_owned()).collect();
            catalog.route(request, Some(&keys))
        }
        Lookup::GetPattern(model, provider) => catalog.get_pattern(model, *provider),
        Lookup::Deployments(model, provider) => {
            Some(catalog.get_deployments_by_pattern(model, *provider))
        }
    };
    matched.map(|entries| {
        entries
            .into_iter()
            .map(|entry| (entry.model_id.unwrap_or_default(), entry.model))
            .collect()
    })
}

fn assert_lookups(catalog: &PatternCatalog, lookups: &[Lookup], expectations: Vec<Expectation>) {
    assert_eq!(lookups.len(), expectations.len());
    for (lookup, expectation) in lookups.iter().zip(expectations) {
        let wanted = expectation.map(|entries| {
            entries
                .into_iter()
                .map(|(id, model)| (id.to_owned(), model.to_owned()))
                .collect::<Vec<Resolved>>()
        });
        assert_eq!(resolve(catalog, lookup), wanted);
    }
}

#[rstest]
#[case::the_more_specific_pattern_is_tried_first(
    vec![("openai/*", "broad", "azure/*"), ("openai/gpt*", "narrow", "bedrock/gpt*")],
    vec![],
    vec![Lookup::Route("openai/gpt-4")],
    vec![Some(vec![("narrow", "bedrock/gpt-4")])],
)]
#[case::registration_order_does_not_beat_specificity(
    vec![("openai/gpt*", "narrow", "bedrock/gpt*"), ("openai/*", "broad", "azure/*")],
    vec![],
    vec![Lookup::Route("openai/gpt-4")],
    vec![Some(vec![("narrow", "bedrock/gpt-4")])],
)]
#[case::equal_specificity_keeps_registration_order(
    vec![("*a", "first", "first/*"), ("a*", "second", "second/*")],
    vec![],
    vec![Lookup::Route("ab")],
    vec![Some(vec![("first", "first/")])],
)]
#[case::every_deployment_of_a_pattern_is_returned_in_order(
    vec![("openai/*", "a", "azure/*"), ("openai/*", "b", "bedrock/*")],
    vec![],
    vec![Lookup::Route("openai/gpt-4")],
    vec![Some(vec![("a", "azure/gpt-4"), ("b", "bedrock/gpt-4")])],
)]
#[case::each_capture_fills_the_next_wildcard(
    vec![("llmengine/fo::*::static::*", "multi", "openai/fo::*::static::*")],
    vec![],
    vec![Lookup::Route("llmengine/fo::hi::static::bye")],
    vec![Some(vec![("multi", "openai/fo::hi::static::bye")])],
)]
#[case::more_captures_than_the_model_wildcards_returns_the_request(
    vec![("llmengine/fo::*::static::*", "fallback", "openai/one*")],
    vec![],
    vec![Lookup::Route("llmengine/fo::hi::static::bye")],
    vec![Some(vec![("fallback", "llmengine/fo::hi::static::bye")])],
)]
#[case::fewer_captures_leave_later_wildcards_literal(
    vec![("openai/*", "partial", "azure/*/*")],
    vec![],
    vec![Lookup::Route("openai/gpt-4")],
    vec![Some(vec![("partial", "azure/gpt-4/*")])],
)]
#[case::a_model_without_a_wildcard_is_unchanged(
    vec![("openai/*", "fixed", "bedrock/fixed")],
    vec![],
    vec![Lookup::Route("openai/gpt-4")],
    vec![Some(vec![("fixed", "bedrock/fixed")])],
)]
#[case::a_pattern_without_a_wildcard_captures_nothing(
    vec![("openai/gpt-4", "literal", "azure/*")],
    vec![],
    vec![Lookup::Route("openai/gpt-4-turbo")],
    vec![Some(vec![("literal", "azure/*")])],
)]
#[case::removing_a_deployment_keeps_the_rest_of_its_pattern(
    vec![("openai/*", "a", "azure/*"), ("openai/*", "b", "bedrock/*")],
    vec!["a"],
    vec![Lookup::Route("openai/gpt-4")],
    vec![Some(vec![("b", "bedrock/gpt-4")])],
)]
#[case::removing_the_last_deployment_drops_the_pattern(
    vec![("openai/*", "only", "azure/*")],
    vec!["only"],
    vec![Lookup::Route("openai/gpt-4")],
    vec![None],
)]
#[case::get_pattern_falls_back_to_the_provider_qualified_name(
    vec![("openai/*", "provider", "azure/*")],
    vec![],
    vec![
        Lookup::GetPattern("gpt-4", Some("openai")),
        Lookup::GetPattern("openai/gpt-4", Some("azure")),
        Lookup::GetPattern("gpt-4", None),
    ],
    vec![
        Some(vec![("provider", "azure/gpt-4")]),
        Some(vec![("provider", "azure/gpt-4")]),
        None,
    ],
)]
#[case::get_deployments_by_pattern_returns_an_empty_list_without_a_match(
    vec![("openai/*", "provider", "azure/*")],
    vec![],
    vec![
        Lookup::Deployments("gpt-4", Some("openai")),
        Lookup::Deployments("claude", Some("anthropic")),
    ],
    vec![Some(vec![("provider", "azure/gpt-4")]), Some(vec![])],
)]
#[case::filtered_model_names_restrict_the_patterns_tried(
    vec![("openai/*", "o", "azure/*"), ("anthropic/*", "a", "fixed")],
    vec![],
    vec![
        Lookup::Filtered("openai/gpt-4", &["anthropic/*"]),
        Lookup::Filtered("openai/gpt-4", &["openai/*"]),
        Lookup::Filtered("anthropic/claude", &["anthropic/*"]),
    ],
    vec![
        None,
        Some(vec![("o", "azure/gpt-4")]),
        Some(vec![("a", "fixed")]),
    ],
)]
fn wildcard_catalog_behaviors(
    #[case] deployments: Vec<Deployment>,
    #[case] removed: Vec<&'static str>,
    #[case] lookups: Vec<Lookup>,
    #[case] expectations: Vec<Expectation>,
) {
    assert_lookups(&catalog(&deployments, &removed), &lookups, expectations);
}

#[rstest]
#[case::an_empty_catalog_has_no_pattern(Boundary::Routes {
    deployments: vec![],
    removed: vec![],
    lookups: vec![Lookup::Route("anything"), Lookup::GetPattern("model", Some("openai"))],
    expectations: vec![None, None],
})]
#[case::an_empty_filter_allows_no_pattern(Boundary::Routes {
    deployments: vec![("openai/*", "a", "azure/*")],
    removed: vec![],
    lookups: vec![Lookup::Filtered("openai/gpt-4", &[])],
    expectations: vec![None],
})]
#[case::an_empty_pattern_matches_every_request_without_substituting(Boundary::Routes {
    deployments: vec![("", "empty", "model*")],
    removed: vec![],
    lookups: vec![Lookup::Route("anything")],
    expectations: vec![Some(vec![("empty", "model*")])],
})]
#[case::literal_metacharacters_do_not_widen_the_match(Boundary::Routes {
    deployments: vec![("meta.llama3*", "llama", "bedrock/meta.llama3*")],
    removed: vec![],
    lookups: vec![Lookup::Route("metaXllama3-70b"), Lookup::Route("meta.llama3-70b")],
    expectations: vec![None, Some(vec![("llama", "bedrock/meta.llama3-70b")])],
})]
#[case::a_backslash_before_a_star_stays_literal(Boundary::Routes {
    deployments: vec![("a\\*b", "slash", "m/*")],
    removed: vec![],
    lookups: vec![Lookup::Route("a\\ZZb"), Lookup::Route("aXXb")],
    expectations: vec![Some(vec![("slash", "m/ZZ")]), None],
})]
#[case::escaped_spaces_count_toward_specificity(Boundary::Routes {
    deployments: vec![("*abcd", "wild", "wild/*"), ("a b*", "space", "space/*")],
    removed: vec![],
    lookups: vec![Lookup::Route("a bxabcd")],
    expectations: vec![Some(vec![("space", "space/xabcd")])],
})]
#[case::unicode_length_is_counted_in_code_points(Boundary::Routes {
    deployments: vec![("a*", "ascii", "ascii/*"), ("*\u{e9}", "unicode", "unicode/*")],
    removed: vec![],
    lookups: vec![Lookup::Route("a\u{e9}")],
    expectations: vec![Some(vec![("ascii", "ascii/\u{e9}")])],
})]
#[case::the_fallback_returns_the_whole_request_of_a_prefix_match(Boundary::Routes {
    deployments: vec![("llmengine/fo::*::static::*", "fallback", "openai/one*")],
    removed: vec![],
    lookups: vec![Lookup::Route("llmengine/fo::hi::static::bye::tail")],
    expectations: vec![Some(vec![
        ("fallback", "llmengine/fo::hi::static::bye::tail"),
    ])],
})]
#[case::removing_an_unknown_model_id_changes_nothing(Boundary::Routes {
    deployments: vec![("openai/*", "a", "azure/*")],
    removed: vec!["unknown"],
    lookups: vec![Lookup::Route("openai/gpt-4")],
    expectations: vec![Some(vec![("a", "azure/gpt-4")])],
})]
#[case::a_candidate_without_a_model_id_is_never_removed(Boundary::UnidentifiedCandidate)]
#[case::a_rejected_compile_keeps_the_pattern_and_its_source(Boundary::RejectedCompile)]
fn wildcard_catalog_boundaries(#[case] boundary: Boundary) {
    match boundary {
        Boundary::Routes {
            deployments,
            removed,
            lookups,
            expectations,
        } => assert_lookups(&catalog(&deployments, &removed), &lookups, expectations),
        Boundary::UnidentifiedCandidate => {
            let mut catalog = PatternCatalog::default();
            let candidate = Candidate {
                model_id: None,
                model: "azure/*".to_owned(),
            };
            catalog.add_pattern("openai/*", candidate).unwrap();
            catalog.remove_deployment("anything");
            let routed = catalog.route("openai/gpt-4", None).unwrap();
            assert_eq!(routed.len(), 1);
            assert_eq!(routed.first().unwrap().model_id, None);
            assert_eq!(routed.first().unwrap().model, "azure/gpt-4");
        }
        Boundary::RejectedCompile => {
            let invalid = "(".to_owned();
            let source = Regex::new(&invalid).unwrap_err();
            let error = PatternCatalogError::Compile {
                pattern: "open(".to_owned(),
                source,
            };
            assert!(error.to_string().contains("\"open(\""));
            assert!(Error::source(&error).is_some());
        }
    }
}
