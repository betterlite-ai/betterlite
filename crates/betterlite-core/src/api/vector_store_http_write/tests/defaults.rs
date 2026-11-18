use super::support::{
    Events, Fixture, RecordingConfig, Reply, Source, Wire, body_bytes, body_json, caller_headers,
    extra_body, params, update_params,
};
use super::{STORE, json, run, target};
use crate::api::vector_store_http_write::params::UpdateParams;
use crate::api::vector_store_http_write::service::{
    ClientKind, DeleteCall, SuppliedClient, UpdateCall, WriteCall, WriteDeps,
};

#[derive(Debug, Clone, Copy)]
pub(super) enum DefaultsCase {
    EmptyParams,
    MetadataNull,
    MetadataAbsent,
    MetadataEmpty,
    ExtraEmpty,
    SuppliedBypassesPool,
    DeleteOmitted,
}

pub(super) async fn write_argument_defaults_case(case: DefaultsCase) {
    let events = Events::new();
    let config = RecordingConfig::new("http://mock.test/v1/vector_stores");
    let wire = Wire::replying(Reply::json(200, STORE), &events);
    let source = Source::new(wire.clone());
    let params = params("http://unused.test");
    let mut fixture = Fixture::new("vector_store_update");
    match case {
        DefaultsCase::EmptyParams => {
            let deps = WriteDeps {
                config: &config,
                source: &source,
                supplied: None,
            };
            let optional = UpdateParams::default();
            let call = WriteCall::Update(UpdateCall {
                target: target("vs_a", &params, None, None),
                optional_params: &optional,
                extra_body: None,
            });
            let value = run(
                &mut fixture,
                &deps,
                ClientKind::Asynchronous,
                &call,
                &events,
            )
            .await
            .expect("the update succeeds");
            assert_eq!(value, json(STORE));
            assert_eq!(source.calls(), vec![None]);
            let requests = wire.requests();
            assert_eq!(requests.len(), 1);
            assert_eq!(body_json(&requests[0]), json("{}"));
            assert_eq!(requests[0].timeout, None);
        }
        DefaultsCase::MetadataNull => {
            let deps = WriteDeps {
                config: &config,
                source: &source,
                supplied: None,
            };
            let optional = update_params(r#"{"metadata":null}"#);
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
            let requests = wire.requests();
            assert_eq!(body_json(&requests[0]), json(r#"{"metadata":null}"#));
        }
        DefaultsCase::MetadataAbsent => {
            let deps = WriteDeps {
                config: &config,
                source: &source,
                supplied: None,
            };
            let optional = update_params(r#"{"name":"notes"}"#);
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
            let requests = wire.requests();
            assert_eq!(body_json(&requests[0]), json(r#"{"name":"notes"}"#));
        }
        DefaultsCase::MetadataEmpty => {
            let deps = WriteDeps {
                config: &config,
                source: &source,
                supplied: None,
            };
            let optional = update_params(r#"{"metadata":{}}"#);
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
            let requests = wire.requests();
            assert_eq!(body_json(&requests[0]), json(r#"{"metadata":{}}"#));
        }
        DefaultsCase::ExtraEmpty => {
            let deps = WriteDeps {
                config: &config,
                source: &source,
                supplied: None,
            };
            let optional = UpdateParams::default();
            let no_headers = caller_headers(&[]);
            let no_body = extra_body("{}");
            let call = WriteCall::Update(UpdateCall {
                target: target("vs_a", &params, Some(&no_headers), None),
                optional_params: &optional,
                extra_body: Some(&no_body),
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
            let requests = wire.requests();
            assert_eq!(requests.len(), 1);
            assert_eq!(body_json(&requests[0]), json("{}"));
            assert_eq!(requests[0].headers.len(), 3);
        }
        DefaultsCase::SuppliedBypassesPool => {
            let supplied = Wire::replying(Reply::json(200, STORE), &events);
            let deps = WriteDeps {
                config: &config,
                source: &source,
                supplied: Some(SuppliedClient {
                    kind: ClientKind::Sync,
                    transport: &supplied,
                }),
            };
            let optional = UpdateParams::default();
            let call = WriteCall::Update(UpdateCall {
                target: target("vs_a", &params, None, None),
                optional_params: &optional,
                extra_body: None,
            });
            run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
                .await
                .expect("the update succeeds");
            assert!(source.calls().is_empty());
            assert_eq!(supplied.requests().len(), 1);
        }
        DefaultsCase::DeleteOmitted => {
            let deps = WriteDeps {
                config: &config,
                source: &source,
                supplied: None,
            };
            let call = WriteCall::Delete(DeleteCall {
                target: target("vs_a", &params, None, None),
                extra_body: None,
            });
            let value = run(
                &mut fixture,
                &deps,
                ClientKind::Asynchronous,
                &call,
                &events,
            )
            .await
            .expect("the delete succeeds");
            assert_eq!(value, json(STORE));
            let requests = wire.requests();
            assert_eq!(requests.len(), 1);
            assert_eq!(body_bytes(&requests[0]), None);
        }
    }
}
