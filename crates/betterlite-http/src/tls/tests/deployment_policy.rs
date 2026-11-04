use std::collections::BTreeMap;
use std::sync::Arc;

use rstest::rstest;

use super::*;
use crate::proxy::ProxiedClient;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;

#[rstest]
fn http_policy_tests_behavior() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(
        "model_list:\n  - model_name: test\n    litellm_params:\n      model: provider/model\n      ssl_verify: false\n",
    )
    .unwrap();
    let deployment = config.model_list.first().unwrap();
    let proxy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    let _client = ProxiedClient::outbound_for_deployment(
        &Arc::new(proxy),
        &config,
        deployment,
        &BTreeMap::new(),
    )
    .unwrap();
}
