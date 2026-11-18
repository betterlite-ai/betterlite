use std::time::Duration;

use super::support::{Events, Fixture, RecordingConfig, Reply, Source, Wire, params};
use super::{STORE, json, run, target};
use crate::api::vector_store_http_write::params::UpdateParams;
use crate::api::vector_store_http_write::service::{
    ClientKind, DeleteCall, SuppliedClient, UpdateCall, WriteCall, WriteDeps,
};

#[derive(Debug, Clone, Copy)]
pub(super) enum ModeCase {
    SuppliedSync,
    SuppliedAsynchronous,
    Mismatch,
    QueryBase,
    LoggingOrder,
}

pub(super) async fn execution_mode_case(case: ModeCase) {
    let events = Events::new();
    let config = RecordingConfig::new("http://mock.test/v1/vector_stores");
    let optional = UpdateParams::default();
    let params = params("http://unused.test");
    let supplied = Wire::replying(Reply::json(200, STORE), &events);
    let pooled = Wire::replying(Reply::json(200, STORE), &events);
    let source = Source::new(pooled.clone());
    let mut fixture = Fixture::new("vector_store_update");
    match case {
        ModeCase::SuppliedSync => {
            let deps = WriteDeps {
                config: &config,
                source: &source,
                supplied: Some(SuppliedClient {
                    kind: ClientKind::Sync,
                    transport: &supplied,
                }),
            };
            let call = WriteCall::Update(UpdateCall {
                target: target("vs_a", &params, None, Some(Duration::from_secs(7))),
                optional_params: &optional,
                extra_body: None,
            });
            let value = run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
                .await
                .expect("the update succeeds");
            assert_eq!(value, json(STORE));
            assert!(source.calls().is_empty());
            let requests = supplied.requests();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].timeout, None);
        }
        ModeCase::SuppliedAsynchronous => {
            let deps = WriteDeps {
                config: &config,
                source: &source,
                supplied: Some(SuppliedClient {
                    kind: ClientKind::Asynchronous,
                    transport: &supplied,
                }),
            };
            let call = WriteCall::Update(UpdateCall {
                target: target("vs_a", &params, None, Some(Duration::from_secs(7))),
                optional_params: &optional,
                extra_body: None,
            });
            run(
                &mut fixture,
                &deps,
                ClientKind::Asynchronous,
                &call,
                &events,
            )
            .await
            .expect("the update succeeds");
            assert!(source.calls().is_empty());
            let requests = supplied.requests();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].timeout, Some(Duration::from_secs(7)));
        }
        ModeCase::Mismatch => {
            let mut params = params("http://unused.test");
            params.ssl_verify = Some(false);
            let deps = WriteDeps {
                config: &config,
                source: &source,
                supplied: Some(SuppliedClient {
                    kind: ClientKind::Sync,
                    transport: &supplied,
                }),
            };
            let call = WriteCall::Update(UpdateCall {
                target: target("vs_a", &params, None, Some(Duration::from_secs(7))),
                optional_params: &optional,
                extra_body: None,
            });
            run(
                &mut fixture,
                &deps,
                ClientKind::Asynchronous,
                &call,
                &events,
            )
            .await
            .expect("the update succeeds");
            assert_eq!(source.calls(), vec![Some(false)]);
            assert!(supplied.requests().is_empty());
            let requests = pooled.requests();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].timeout, Some(Duration::from_secs(7)));
        }
        ModeCase::QueryBase => {
            let config = RecordingConfig::new("https://mock.test/vector_stores?api-version=v1");
            let deps = WriteDeps {
                config: &config,
                source: &source,
                supplied: None,
            };
            let call = WriteCall::Update(UpdateCall {
                target: target("vs_a", &params, None, None),
                optional_params: &optional,
                extra_body: None,
            });
            run(
                &mut fixture,
                &deps,
                ClientKind::Asynchronous,
                &call,
                &events,
            )
            .await
            .expect("the update succeeds");
            let requests = pooled.requests();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].url.path(), "/vector_stores");
            assert_eq!(requests[0].url.query(), Some("api-version=v1/vs_a"));
            assert_eq!(
                requests[0].url.as_str(),
                "https://mock.test/vector_stores?api-version=v1/vs_a"
            );
        }
        ModeCase::LoggingOrder => {
            let deps = WriteDeps {
                config: &config,
                source: &source,
                supplied: None,
            };
            let update = WriteCall::Update(UpdateCall {
                target: target("vs_a", &params, None, None),
                optional_params: &optional,
                extra_body: None,
            });
            run(
                &mut fixture,
                &deps,
                ClientKind::Asynchronous,
                &update,
                &events,
            )
            .await
            .expect("the update succeeds");
            assert_eq!(events.recorded(), ["log", "send"]);
            assert!(fixture.logged("complete_input_dict").is_some());
            let delete = WriteCall::Delete(DeleteCall {
                target: target("vs_a", &params, None, None),
                extra_body: None,
            });
            run(
                &mut fixture,
                &deps,
                ClientKind::Asynchronous,
                &delete,
                &events,
            )
            .await
            .expect("the delete succeeds");
            assert_eq!(events.recorded(), ["log", "send", "log", "send"]);
            assert!(fixture.logged("complete_input_dict").is_none());
        }
    }
}
