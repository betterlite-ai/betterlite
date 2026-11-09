use crate::datadog::apm::NullTracer;
use crate::datadog::payloads::legacy::legacy_v1_logging_to_datadog;
use crate::datadog::payloads::standard::standard_logging_to_datadog;
use crate::datadog::payloads::status::DataDogStatus;

use super::support::{default_environment, default_kwargs, json, legacy_event, standard_payload};

#[test]
fn datadog_config_defaults_behavior() {
    let environment = default_environment();
    let tags = "env:unknown,service:litellm-server,version:unknown,HOSTNAME:,POD_NAME:unknown";
    let built = standard_logging_to_datadog(
        &standard_payload(),
        DataDogStatus::Info,
        &environment,
        &NullTracer,
    )
    .unwrap();
    assert_eq!(built.ddsource, "litellm");
    assert_eq!(built.service, "litellm-server");
    assert_eq!(built.hostname, "");
    assert_eq!(built.ddtags, tags);
    let kwargs = default_kwargs();
    let response = json(r#"{"usage": {}}"#);
    let legacy =
        legacy_v1_logging_to_datadog(&legacy_event(&kwargs, &response), &environment, &NullTracer)
            .unwrap();
    assert_eq!(legacy.ddsource, "litellm");
    assert_eq!(legacy.service, "litellm-server");
    assert_eq!(legacy.hostname, "");
    assert_eq!(legacy.ddtags, tags);
}
