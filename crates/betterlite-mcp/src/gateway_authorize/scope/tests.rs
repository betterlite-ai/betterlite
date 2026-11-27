#![cfg(test)]

use super::*;
use crate::gateway_authorize::tests::support::{
    BASE, DELEGATED_SERVER, M2M_SERVER, OAUTH_SERVER, Reach, flow, registry,
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
    let resource = format!("{BASE}/srv-a/mcp");
    let scoped = resolve_scoped_resource_server(&registry, BASE, Some(&resource)).unwrap();
    assert_eq!(scoped.fields.server_id, "srv-a");
}

#[test]
fn every_other_resource_shape_stays_unscoped() {
    let registry = registry(&[OAUTH_SERVER, DELEGATED_SERVER]);
    let shapes = [
        None,
        Some(format!("{BASE}/mcp")),
        Some(BASE.to_owned()),
        Some("https://other.test/srv-a/mcp".to_owned()),
        Some(format!("{BASE}/srv-a/srv-c/mcp")),
        Some(format!("{BASE}/unknown/mcp")),
        Some(format!("{BASE}/srv-c/mcp")),
        Some(format!("{BASE}/srv-a")),
    ];
    for resource in shapes {
        let scoped = resolve_scoped_resource_server(&registry, BASE, resource.as_deref());
        assert!(scoped.is_none(), "{resource:?}");
    }
}

#[tokio::test]
async fn flow_target_reports_its_state() {
    let registry = registry(&[OAUTH_SERVER, M2M_SERVER, DELEGATED_SERVER]);
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
