#![cfg(test)]

use rstest::rstest;

use super::*;
use crate::gateway_authorize::tests::support::{
    BASE, DELEGATED_SERVER, M2M_SERVER, OAUTH_SERVER, PREFIX_SERVER, RELAY_SERVER, Reach, flow,
    registry,
};

#[test]
fn the_base_url_alone_is_the_proxy_api_resource() {
    assert!(is_proxy_api_resource(BASE, Some(BASE)));
    assert!(!is_proxy_api_resource(BASE, None));
    assert!(!is_proxy_api_resource(BASE, Some("https://other.test")));
    assert!(!is_proxy_api_resource(BASE, Some("not a url")));
}

#[test]
fn a_server_resource_selects_the_gateway_server() {
    let registry = registry(&[OAUTH_SERVER, DELEGATED_SERVER]);
    let resource = format!("{BASE}/alpha/mcp");
    let scoped = resolve_scoped_resource_server(&registry, BASE, Some(&resource)).unwrap();
    assert_eq!(scoped.fields.server_id, "srv-a");
}

#[test]
fn a_relayed_gateway_server_is_still_scoped() {
    let registry = registry(&[RELAY_SERVER]);
    let resource = format!("{BASE}/relay/mcp");
    let scoped = resolve_scoped_resource_server(&registry, BASE, Some(&resource)).unwrap();
    assert_eq!(scoped.fields.server_id, "srv-r");
}

#[test]
fn a_resource_sharing_only_the_base_path_prefix_stays_unscoped() {
    let registry = registry(&[PREFIX_SERVER]);
    let resource = "https://proxy.test/tenantx/mcp";
    let scoped =
        resolve_scoped_resource_server(&registry, "https://proxy.test/tenant", Some(resource));
    assert!(scoped.is_none());
}

#[rstest]
#[case::absent(None)]
#[case::aggregate(Some("https://proxy.test:8443/mcp"))]
#[case::proxy_api(Some("https://proxy.test:8443"))]
#[case::foreign_host(Some("https://other.test/alpha/mcp"))]
#[case::several_servers(Some("https://proxy.test:8443/alpha/gamma/mcp"))]
#[case::unknown_server(Some("https://proxy.test:8443/unknown/mcp"))]
#[case::delegated_only(Some("https://proxy.test:8443/gamma/mcp"))]
#[case::without_suffix(Some("https://proxy.test:8443/alpha"))]
fn every_other_resource_shape_stays_unscoped(#[case] resource: Option<&'static str>) {
    let registry = registry(&[OAUTH_SERVER, DELEGATED_SERVER]);
    let scoped = resolve_scoped_resource_server(&registry, BASE, resource);
    assert!(scoped.is_none(), "{resource:?}");
}

#[tokio::test]
async fn flow_target_reports_its_state() {
    let registry = registry(&[OAUTH_SERVER, M2M_SERVER, DELEGATED_SERVER, RELAY_SERVER]);
    assert!(matches!(
        flow_target(&registry, &flow(), &Reach(true)).await,
        FlowTarget::Unscoped
    ));
    assert!(matches!(
        flow_target(&registry, &scoped_flow("srv-a"), &Reach(true)).await,
        FlowTarget::Interactive(_)
    ));
    assert!(matches!(
        flow_target(&registry, &scoped_flow("srv-b"), &Reach(true)).await,
        FlowTarget::M2m(_)
    ));
    assert!(matches!(
        flow_target(&registry, &scoped_flow("srv-r"), &Reach(true)).await,
        FlowTarget::Interactive(_)
    ));
    assert!(matches!(
        flow_target(&registry, &scoped_flow("srv-c"), &Reach(true)).await,
        FlowTarget::Stale
    ));
    assert!(matches!(
        flow_target(&registry, &scoped_flow("srv-z"), &Reach(true)).await,
        FlowTarget::Stale
    ));
    assert!(matches!(
        flow_target(&registry, &scoped_flow("srv-a"), &Reach(false)).await,
        FlowTarget::Stale
    ));
}

fn scoped_flow(server_id: &str) -> ConnectFlow {
    ConnectFlow {
        resource_server_id: Some(server_id.to_owned()),
        ..flow()
    }
}
