use std::collections::BTreeMap;

use betterlite_openai::api::container_contracts::config::{
    BaseContainerConfig, ContainerListQuery, ContainerRequestContext,
};
use betterlite_openai::api::container_contracts::error::ContainerError;
use betterlite_openai::openai_base_config::Object;

use super::fixture::{container_config, object};
use crate::openai_resource_dispatch::containers::{Call, Dispatch, Outcome};
use crate::openai_resource_dispatch::{
    OpenAiResourceError, OpenAiResourceOperation, OpenAiResourceResult, dispatch,
};

const API_BASE: &str = "https://api.containers.test/v1";

async fn run(
    config: &dyn BaseContainerConfig,
    context: ContainerRequestContext<'_>,
    call: Call<'_>,
) -> Result<OpenAiResourceResult, OpenAiResourceError> {
    let operation = OpenAiResourceOperation::Containers(Box::new(Dispatch {
        config,
        context,
        call,
    }));
    dispatch(operation).await
}

fn request_result(result: Result<OpenAiResourceResult, OpenAiResourceError>) -> (String, Object) {
    let Ok(OpenAiResourceResult::Containers(Outcome::Request { url, query })) = result else {
        panic!("the request transform returns its url and query");
    };
    (url, query)
}

async fn create_row() {
    let config = container_config();
    let params = object(r#"{"file_ids":["file-1"]}"#);
    let headers = BTreeMap::new();
    let context = ContainerRequestContext {
        litellm_params: &params,
        headers: &headers,
    };
    let request = run(
        &config,
        context,
        Call::Create {
            name: "box",
            params: &params,
        },
    )
    .await;
    let Ok(OpenAiResourceResult::Containers(Outcome::Create(body))) = request else {
        panic!("the create request transform returns its body");
    };
    assert_eq!(body, object(r#"{"name":"box","file_ids":["file-1"]}"#));
}

async fn list_row() {
    let config = container_config();
    let params = object("{}");
    let headers = BTreeMap::new();
    let context = ContainerRequestContext {
        litellm_params: &params,
        headers: &headers,
    };
    let extra = object(r#"{"filter":"mine"}"#);
    let query = ContainerListQuery {
        after: Some("cntr_0"),
        limit: Some(2),
        order: Some("desc"),
        extra_query: Some(&extra),
    };
    let request = run(
        &config,
        context,
        Call::List {
            api_base: API_BASE,
            query,
        },
    )
    .await;
    let (url, query) = request_result(request);
    assert_eq!(url, API_BASE);
    assert_eq!(
        query,
        object(r#"{"after":"cntr_0","limit":"2","order":"desc","filter":"mine"}"#)
    );
}

async fn retrieve_row() {
    let config = container_config();
    let params = object("{}");
    let headers = BTreeMap::new();
    let context = ContainerRequestContext {
        litellm_params: &params,
        headers: &headers,
    };
    let request = run(
        &config,
        context,
        Call::Retrieve {
            container_id: "cntr_1",
            api_base: API_BASE,
        },
    )
    .await;
    let (url, query) = request_result(request);
    assert_eq!(url, "https://api.containers.test/v1/cntr_1");
    assert_eq!(query, object("{}"));
}

async fn delete_row() {
    let config = container_config();
    let params = object("{}");
    let headers = BTreeMap::new();
    let context = ContainerRequestContext {
        litellm_params: &params,
        headers: &headers,
    };
    let request = run(
        &config,
        context,
        Call::Delete {
            container_id: "cntr_1",
            api_base: API_BASE,
        },
    )
    .await;
    let (url, query) = request_result(request);
    assert_eq!(url, "https://api.containers.test/v1/cntr_1");
    assert_eq!(query, object("{}"));
}

async fn file_list_row() {
    let config = container_config();
    let params = object("{}");
    let headers = BTreeMap::new();
    let context = ContainerRequestContext {
        litellm_params: &params,
        headers: &headers,
    };
    let query = ContainerListQuery {
        after: None,
        limit: Some(1),
        order: None,
        extra_query: None,
    };
    let request = run(
        &config,
        context,
        Call::FileList {
            container_id: "cntr_1",
            api_base: API_BASE,
            query,
        },
    )
    .await;
    let (url, query) = request_result(request);
    assert_eq!(url, "https://api.containers.test/v1/cntr_1/files");
    assert_eq!(query, object(r#"{"limit":"1"}"#));
}

async fn file_content_row() {
    let config = container_config();
    let params = object("{}");
    let headers = BTreeMap::new();
    let context = ContainerRequestContext {
        litellm_params: &params,
        headers: &headers,
    };
    let request = run(
        &config,
        context,
        Call::FileContent {
            container_id: "cntr_1",
            file_id: "cfile_1",
            api_base: API_BASE,
        },
    )
    .await;
    let (url, query) = request_result(request);
    assert_eq!(
        url,
        "https://api.containers.test/v1/cntr_1/files/cfile_1/content"
    );
    assert_eq!(query, object("{}"));
    let reply = run(&config, context, Call::FileContentReply(b"\x00\x01raw\xff")).await;
    let Ok(OpenAiResourceResult::Containers(Outcome::FileContent(content))) = reply else {
        panic!("the file content reply keeps its bytes");
    };
    assert_eq!(content.as_ref(), b"\x00\x01raw\xff");
}

async fn error_row() {
    let config = container_config();
    let params = object("{}");
    let headers = BTreeMap::new();
    let context = ContainerRequestContext {
        litellm_params: &params,
        headers: &headers,
    };
    let result = run(
        &config,
        context,
        Call::Retrieve {
            container_id: "..",
            api_base: API_BASE,
        },
    )
    .await;
    let Err(error) = result else {
        panic!("a dot path segment is a leaf error");
    };
    assert!(matches!(
        error,
        OpenAiResourceError::Containers(ContainerError::PathSegment { .. })
    ));
}

pub(crate) async fn check() {
    create_row().await;
    list_row().await;
    retrieve_row().await;
    delete_row().await;
    file_list_row().await;
    file_content_row().await;
    error_row().await;
}
