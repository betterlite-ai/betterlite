use crate::websocket_driver::service::ServiceInput;

#[path = "../helpers/core_ops/responses_ws_managed.rs"]
pub(crate) mod managed;

pub(crate) type ManagedInput = ServiceInput<managed::Input>;
