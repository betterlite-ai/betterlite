#![cfg(test)]

use super::{CATALOG, RouteCatalog};
use crate::routes::{CallType, RouteMetadataError, get_routes_for_call_type};

#[rstest::rstest]
#[case::pinned_catalog(CATALOG, None)]
#[case::malformed(b"{", Some("fixture"))]
#[case::unknown_call_type(
    br#"{"source":"test","call_types":["unknown"],"routes":[]}"#,
    Some("fixture")
)]
#[case::empty_route_types(
    br#"{"source":"test","call_types":[],"routes":[{"route":"/test","call_types":[]}]}"#,
    Some("empty")
)]
#[case::undeclared_call_type(br#"{"source":"test","call_types":[],"routes":[{"route":"/test","call_types":["completion"]}]}"#, Some("undeclared"))]
#[case::oversized(&vec![b' '; 0x0002_0001], Some("large"))]
fn call_route_metadata_route_metadata_tests_behavior(
    #[case] bytes: &[u8],
    #[case] error: Option<&str>,
) {
    let result = RouteCatalog::parse(bytes);
    match error {
        Some("fixture") => assert!(matches!(result, Err(RouteMetadataError::Fixture(_)))),
        Some("empty") => assert!(matches!(result, Err(RouteMetadataError::EmptyRouteTypes))),
        Some("undeclared") => assert!(matches!(
            result,
            Err(RouteMetadataError::UndeclaredCallType)
        )),
        Some("large") => assert!(matches!(result, Err(RouteMetadataError::CatalogTooLong))),
        Some(other) => panic!("unexpected test error: {other}"),
        None => {
            let catalog = result.unwrap();
            assert_eq!(catalog.call_types.len(), 153);
            assert_eq!(catalog.routes.len(), 120);
            assert_eq!(
                catalog.source,
                "litellm@d665dc1a6315847fcad6cc2c1264ceb47c06abce"
            );
            assert_eq!(
                get_routes_for_call_type(CallType::Completion).unwrap(),
                [
                    "/chat/completions",
                    "/v1/chat/completions",
                    "/engines/{model}/chat/completions",
                    "/openai/deployments/{model}/chat/completions"
                ]
            );
            assert!(
                get_routes_for_call_type(CallType::AcreateSkill)
                    .unwrap()
                    .is_empty()
            );
        }
    }
}
