use std::sync::Arc;

use rmcp::Service as _;
use rmcp::model::ProtocolVersion;

use crate::client::sdk_runtime::convert::{DEFAULT_CLIENT_NAME, DEFAULT_CLIENT_VERSION};
use crate::client::sdk_runtime::handler::progress::ProgressRegistry;
use crate::client::sdk_runtime::handler::{SdkClientHandler, ServerRequestHandlers};

use super::server::{TestElicitation, TestSampling};

#[test]
fn the_client_info_names_the_sdk_identity_and_the_configured_callbacks() {
    let handler = SdkClientHandler {
        handlers: ServerRequestHandlers {
            sampling: Some(Arc::new(TestSampling)),
            elicitation: Some(Arc::new(TestElicitation)),
        },
        progress: Arc::new(ProgressRegistry::default()),
    };
    let info = handler.get_info();
    assert_eq!(
        info.protocol_version,
        ProtocolVersion::LATEST_WITH_INITIALIZE
    );
    assert_eq!(info.client_info.name, DEFAULT_CLIENT_NAME);
    assert_eq!(info.client_info.version, DEFAULT_CLIENT_VERSION);
    assert!(
        info.capabilities.sampling.is_some(),
        "the sampling responder advertises its capability"
    );
    assert!(
        info.capabilities.elicitation.is_some(),
        "the elicitation responder advertises its capability"
    );
}
