use std::collections::BTreeMap;

use betterlite_core::api::vector_store_http_write::config::WriteParams;
use betterlite_core::api::vector_store_read::{
    TransportRequest as ReadTransport, VectorStoreListRequest, VectorStoreReadRequest,
};
use betterlite_core::api::vector_store_write::{
    TransportRequest as WriteTransport, VectorStoreWriteRequest,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::{DeleteWire, ListWire, RetrieveWire, UpdateWire};

// the kwargs and overrides every read carries, with extra_query accepted and not forwarded
fn read_transport<'wire>(
    kwargs: &'wire IndexMap<String, Json>,
    extra_headers: Option<&'wire BTreeMap<String, String>>,
    extra_query: Option<&'wire IndexMap<String, Json>>,
    extra_body: Option<&'wire IndexMap<String, Json>>,
    timeout: Option<f64>,
    custom_llm_provider: Option<&'wire str>,
    params: &'wire WriteParams,
) -> ReadTransport<'wire> {
    ReadTransport {
        extra_headers,
        extra_query,
        extra_body,
        timeout,
        custom_llm_provider,
        params,
        kwargs,
    }
}

// the call the read core's retrieve takes, borrowing the wire and the parameters for as long as it runs
pub(crate) fn retrieve_request<'wire>(
    wire: &'wire RetrieveWire,
    params: &'wire WriteParams,
) -> VectorStoreReadRequest<'wire> {
    VectorStoreReadRequest {
        transport: read_transport(
            &wire.kwargs,
            wire.extra_headers.as_ref(),
            wire.extra_query.as_ref(),
            wire.extra_body.as_ref(),
            wire.timeout,
            wire.custom_llm_provider.as_deref(),
            params,
        ),
        vector_store_id: &wire.vector_store_id,
    }
}

// list's four arguments keep the signature's defaults: an absent field is None and an explicit null is Some(None)
pub(crate) fn list_request<'wire>(
    wire: &'wire ListWire,
    params: &'wire WriteParams,
) -> VectorStoreListRequest<'wire> {
    VectorStoreListRequest {
        transport: read_transport(
            &wire.kwargs,
            wire.extra_headers.as_ref(),
            wire.extra_query.as_ref(),
            wire.extra_body.as_ref(),
            wire.timeout,
            wire.custom_llm_provider.as_deref(),
            params,
        ),
        after: wire.after.as_deref(),
        before: wire.before.as_deref(),
        limit: wire.limit,
        order: wire.order.as_ref().map(|order| order.as_deref()),
    }
}

// the kwargs and overrides every write carries, over the id the write addresses
fn write_transport<'wire>(
    vector_store_id: &'wire str,
    kwargs: &'wire IndexMap<String, Json>,
    extra_headers: Option<&'wire BTreeMap<String, String>>,
    extra_query: Option<&'wire IndexMap<String, Json>>,
    extra_body: Option<&'wire IndexMap<String, Json>>,
    timeout: Option<f64>,
    custom_llm_provider: Option<&'wire str>,
    params: &'wire WriteParams,
) -> WriteTransport<'wire> {
    WriteTransport {
        vector_store_id,
        extra_headers,
        extra_query,
        extra_body,
        timeout,
        custom_llm_provider,
        params,
        kwargs,
    }
}

// the call the write core's update takes; a None field is left out and an empty metadata still travels
pub(crate) fn update_request<'wire>(
    wire: &'wire UpdateWire,
    params: &'wire WriteParams,
) -> VectorStoreWriteRequest<'wire> {
    VectorStoreWriteRequest {
        transport: write_transport(
            &wire.vector_store_id,
            &wire.kwargs,
            wire.extra_headers.as_ref(),
            wire.extra_query.as_ref(),
            wire.extra_body.as_ref(),
            wire.timeout,
            wire.custom_llm_provider.as_deref(),
            params,
        ),
        name: wire.name.as_deref(),
        expires_after: wire.expires_after.as_ref(),
        metadata: wire.metadata.as_ref(),
    }
}

// the call the write core's delete takes: the id alone, so every create field stays out of it
pub(crate) fn delete_request<'wire>(
    wire: &'wire DeleteWire,
    params: &'wire WriteParams,
) -> VectorStoreWriteRequest<'wire> {
    VectorStoreWriteRequest {
        transport: write_transport(
            &wire.vector_store_id,
            &wire.kwargs,
            wire.extra_headers.as_ref(),
            wire.extra_query.as_ref(),
            wire.extra_body.as_ref(),
            wire.timeout,
            wire.custom_llm_provider.as_deref(),
            params,
        ),
        name: None,
        expires_after: None,
        metadata: None,
    }
}
