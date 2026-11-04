#![cfg(test)]
use super::{
    CallType, RouteMetadataError, get_call_types_for_route, get_routes_for_call_type, parse_route,
    route_catalog,
};

#[rstest::rstest]
#[case::catalog("catalog")]
#[case::empty_path("empty_path")]
#[case::bounded_segments("bounded_segments")]
#[case::exact_precedence("exact")]
#[case::chat_tools_exact("chat_tools_exact")]
#[case::chat_tools_versioned("chat_tools_versioned")]
#[case::chat_tools_engine("chat_tools_engine")]
#[case::chat_tools_deployment("chat_tools_deployment")]
#[case::single_placeholder("placeholder")]
#[case::suffixed_segment("suffix")]
#[case::multi_segment_tail("tail")]
#[case::empty_suffix_value("empty_suffix")]
#[case::trailing_slashes("trailing")]
#[case::inverse_lookup("inverse")]
#[case::unknown_route("unknown")]
#[case::bounded_route("bounds")]
fn call_route_metadata_route_metadata_tests_behavior(#[case] behavior: &str) {
    match behavior {
        "catalog" => {
            let catalog = route_catalog().unwrap();
            assert_eq!(catalog.call_types.len(), 153);
            assert_eq!(catalog.routes.len(), 120);
            assert!(
                catalog
                    .routes
                    .iter()
                    .all(|entry| !entry.call_types.is_empty())
            );
            assert!(
                catalog
                    .source
                    .contains("d665dc1a6315847fcad6cc2c1264ceb47c06abce")
            );
        }
        "empty_path" => assert!(parse_route("").unwrap().is_empty()),
        "bounded_segments" => {
            let route = format!("/{}", vec!["x"; 257].join("/"));
            assert!(matches!(
                parse_route(&route),
                Err(RouteMetadataError::TooManySegments)
            ));
        }
        "exact" => assert_eq!(
            get_call_types_for_route("/videos/characters").unwrap(),
            Some(vec![
                CallType::AvideoCreateCharacter,
                CallType::VideoCreateCharacter
            ]),
        ),
        "chat_tools_exact" => assert_eq!(
            get_call_types_for_route("/chat/completions").unwrap(),
            Some(vec![CallType::Acompletion, CallType::Completion]),
        ),
        "chat_tools_versioned" => assert_eq!(
            get_call_types_for_route("/v1/chat/completions").unwrap(),
            Some(vec![CallType::Acompletion, CallType::Completion]),
        ),
        "chat_tools_engine" => assert_eq!(
            get_call_types_for_route("/engines/gpt/chat/completions").unwrap(),
            Some(vec![CallType::Acompletion, CallType::Completion]),
        ),
        "chat_tools_deployment" => assert_eq!(
            get_call_types_for_route("/openai/deployments/deploy/chat/completions").unwrap(),
            Some(vec![CallType::Acompletion, CallType::Completion]),
        ),
        "placeholder" => assert!(
            get_call_types_for_route("/v1/threads/thread-1")
                .unwrap()
                .is_some()
        ),
        "suffix" => assert!(
            get_call_types_for_route("/models/gpt:generateContent")
                .unwrap()
                .is_some()
        ),
        "tail" => assert!(
            get_call_types_for_route("/models/org/model:generateContent")
                .unwrap()
                .is_some()
        ),
        "empty_suffix" => assert_eq!(
            get_call_types_for_route("/models/:generateContent").unwrap(),
            None
        ),
        "trailing" => assert!(
            get_call_types_for_route("/v1/chat/completions/")
                .unwrap()
                .is_some()
        ),
        "inverse" => assert!(
            get_routes_for_call_type(CallType::Completion)
                .unwrap()
                .contains(&"/chat/completions".to_owned())
        ),
        "unknown" => assert_eq!(get_call_types_for_route("/no-such-route").unwrap(), None),
        "bounds" => assert!(matches!(
            parse_route(&"x".repeat(4097)),
            Err(RouteMetadataError::RouteTooLong)
        )),
        _ => assert_eq!(behavior, "bounds"),
    }
}

#[rstest::rstest]
#[case::suffixed_multi_tail("suffixed-multi-tail")]
fn replays_the_route_pattern_corpus(#[case] seed: &str) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fuzz/corpus/route_pattern")
        .join(seed);
    let bytes = std::fs::read(path).unwrap();
    let route = std::str::from_utf8(&bytes).unwrap();
    drop(parse_route(route));
    drop(get_call_types_for_route(route));
}
