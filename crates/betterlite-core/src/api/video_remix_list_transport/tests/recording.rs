use std::collections::BTreeSet;
use std::io;
use std::sync::{Mutex, PoisonError};

use betterlite_models::decisions::Json;
use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use secrecy::ExposeSecret as _;

use super::support::{Events, LIST_PAGE, video_object};
use crate::api::video_records::VideoObject;
use crate::api::video_remix_list_transport::config::VideoRemixListConfig;
use crate::api::video_remix_list_transport::error::{ConfigFailure, ErrorHeaders};
use crate::api::video_remix_list_transport::records::{
    Headers, Params, VideoEnvironment, VideoListQuery, VideoReply, VideoRequestContext,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Step {
    Url,
    RemixRequest,
    RemixResponse,
    ListRequest,
    ListResponse,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RemixInput {
    pub(crate) video_id: String,
    pub(crate) prompt: String,
    pub(crate) extra_body: Option<Params>,
    pub(crate) api_base: String,
    pub(crate) headers: Headers,
    pub(crate) litellm_params: Params,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ListQuery {
    pub(crate) after: Option<String>,
    pub(crate) limit: Option<i64>,
    pub(crate) order: Option<String>,
    pub(crate) extra_query: Option<Params>,
    pub(crate) api_base: String,
    pub(crate) headers: Headers,
    pub(crate) litellm_params: Params,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReplyRecord {
    pub(crate) status: u16,
    pub(crate) body: Vec<u8>,
    pub(crate) provider: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ErrorCall {
    pub(crate) message: String,
    pub(crate) status_code: u16,
    pub(crate) headers: ErrorHeaders,
}

fn push<T>(cell: &Mutex<Vec<T>>, value: T) {
    let mut held = cell.lock().unwrap_or_else(PoisonError::into_inner);
    held.push(value);
}

fn list_query(query: VideoListQuery<'_>, context: &VideoRequestContext<'_>) -> ListQuery {
    ListQuery {
        after: query.after.map(str::to_owned),
        limit: query.limit,
        order: query.order.map(str::to_owned),
        extra_query: query.extra_query.cloned(),
        api_base: context.api_base.to_owned(),
        headers: context.headers.clone(),
        litellm_params: context.litellm_params.clone(),
    }
}

// The fake BaseVideoConfig: it records every call and every argument a transform reads, and fails where told
pub(crate) struct RecordingConfig {
    events: Events,
    base: String,
    remix_url: String,
    remix_body: Params,
    remix_object: VideoObject,
    list_url: String,
    list_params: Params,
    list_page: Params,
    failures: BTreeSet<Step>,
    api_keys: Mutex<Vec<Option<String>>>,
    models: Mutex<Vec<String>>,
    validated: Mutex<Vec<Headers>>,
    validated_params: Mutex<Vec<Option<Params>>>,
    urls: Mutex<Vec<(String, Option<String>)>>,
    url_params: Mutex<Vec<Params>>,
    remix_inputs: Mutex<Vec<RemixInput>>,
    list_inputs: Mutex<Vec<ListQuery>>,
    replies: Mutex<Vec<ReplyRecord>>,
    errors: Mutex<Vec<ErrorCall>>,
}

impl RecordingConfig {
    pub(crate) fn new(events: &Events) -> Self {
        Self {
            events: events.clone(),
            base: String::from("https://base.test/v1"),
            remix_url: String::from("https://api.test/v1/videos/video_a/remix"),
            remix_body: serde_json::from_str(r#"{"prompt":"make it slow"}"#)
                .expect("the remix body fixture is valid"),
            remix_object: video_object("video_out", "queued"),
            list_url: String::from("https://api.test/v1/videos"),
            list_params: Params::new(),
            list_page: serde_json::from_str(LIST_PAGE).expect("the list page fixture is valid"),
            failures: BTreeSet::new(),
            api_keys: Mutex::default(),
            models: Mutex::default(),
            validated: Mutex::default(),
            validated_params: Mutex::default(),
            urls: Mutex::default(),
            url_params: Mutex::default(),
            remix_inputs: Mutex::default(),
            list_inputs: Mutex::default(),
            replies: Mutex::default(),
            errors: Mutex::default(),
        }
    }

    pub(crate) fn failing(mut self, step: Step) -> Self {
        self.failures.insert(step);
        self
    }

    pub(crate) fn with_base(mut self, base: &str) -> Self {
        self.base = base.to_owned();
        self
    }

    pub(crate) fn with_remix_url(mut self, url: &str) -> Self {
        self.remix_url = url.to_owned();
        self
    }

    pub(crate) fn with_list_url(mut self, url: &str) -> Self {
        self.list_url = url.to_owned();
        self
    }

    pub(crate) fn with_list_params(mut self, params: Params) -> Self {
        self.list_params = params;
        self
    }

    pub(crate) fn with_remix_object(mut self, object: VideoObject) -> Self {
        self.remix_object = object;
        self
    }

    pub(crate) fn base(&self) -> &str {
        &self.base
    }

    pub(crate) fn remix_url(&self) -> &str {
        &self.remix_url
    }

    pub(crate) fn remix_body(&self) -> &Params {
        &self.remix_body
    }

    pub(crate) fn list_url(&self) -> &str {
        &self.list_url
    }

    pub(crate) fn list_params(&self) -> &Params {
        &self.list_params
    }

    pub(crate) fn list_page(&self) -> &Params {
        &self.list_page
    }

    pub(crate) fn events(&self) -> Vec<String> {
        self.events.recorded()
    }

    pub(crate) fn api_keys(&self) -> Vec<Option<String>> {
        self.api_keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn models(&self) -> Vec<String> {
        self.models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn validated(&self) -> Vec<Headers> {
        self.validated
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn validated_params(&self) -> Vec<Option<Params>> {
        self.validated_params
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn urls(&self) -> Vec<(String, Option<String>)> {
        self.urls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn url_params(&self) -> Vec<Params> {
        self.url_params
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn remix_inputs(&self) -> Vec<RemixInput> {
        self.remix_inputs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn list_inputs(&self) -> Vec<ListQuery> {
        self.list_inputs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn replies(&self) -> Vec<ReplyRecord> {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn errors(&self) -> Vec<ErrorCall> {
        self.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn check(&self, step: Step) -> Result<(), ConfigFailure> {
        if self.fails(step) {
            return Err(Self::failure(step));
        }
        Ok(())
    }

    fn list_request(&self, seen: ListQuery) -> Result<(String, Params), ConfigFailure> {
        self.events.push("list_request");
        push(&self.list_inputs, seen);
        self.check(Step::ListRequest)?;
        Ok((self.list_url.clone(), self.list_params.clone()))
    }

    fn record_transform(
        &self,
        reply: &mut VideoReply<'_>,
        name: &str,
        marker: &str,
        step: Step,
    ) -> Result<(), ConfigFailure> {
        self.events.push(name);
        let record = ReplyRecord {
            status: reply.raw_response.status,
            body: reply.raw_response.body.clone(),
            provider: reply.custom_llm_provider.map(str::to_owned),
        };
        push(&self.replies, record);
        reply.logging_obj.model_call_details.insert(
            String::from("video_transform"),
            Json::Text(marker.to_owned()),
        );
        self.check(step)
    }

    fn failure(step: Step) -> ConfigFailure {
        ConfigFailure {
            source: Box::new(io::Error::other(format!(
                "the fake config fails at {step:?}"
            ))),
        }
    }

    fn fails(&self, step: Step) -> bool {
        self.failures.contains(&step)
    }
}

impl VideoRemixListConfig for RecordingConfig {
    fn validate_environment(&self, request: VideoEnvironment<'_>) -> Headers {
        self.events.push("validate");
        self.models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.model.to_owned());
        self.api_keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.api_key.map(|key| key.expose_secret().to_owned()));
        self.validated
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.headers.clone());
        self.validated_params
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.litellm_params.cloned());
        let mut headers = request.headers.clone();
        headers.insert("Authorization".to_owned(), "Bearer sk-provider".to_owned());
        headers.insert("Content-Type".to_owned(), "application/json".to_owned());
        headers.insert("X-Trace".to_owned(), "config-trace".to_owned());
        headers
    }

    fn get_complete_url(
        &self,
        model: &str,
        api_base: Option<&str>,
        litellm_params: &Params,
    ) -> Result<String, ConfigFailure> {
        self.events.push("url");
        self.urls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((model.to_owned(), api_base.map(str::to_owned)));
        self.url_params
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(litellm_params.clone());
        if self.fails(Step::Url) {
            return Err(Self::failure(Step::Url));
        }
        Ok(self.base.clone())
    }

    fn transform_video_remix_request(
        &self,
        video_id: &str,
        prompt: &str,
        extra_body: Option<&Params>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), ConfigFailure> {
        self.events.push("remix_request");
        push(
            &self.remix_inputs,
            RemixInput {
                video_id: video_id.to_owned(),
                prompt: prompt.to_owned(),
                extra_body: extra_body.cloned(),
                api_base: context.api_base.to_owned(),
                headers: context.headers.clone(),
                litellm_params: context.litellm_params.clone(),
            },
        );
        self.check(Step::RemixRequest)?;
        Ok((self.remix_url.clone(), self.remix_body.clone()))
    }

    fn transform_video_remix_response(
        &self,
        mut reply: VideoReply<'_>,
    ) -> Result<VideoObject, ConfigFailure> {
        self.record_transform(&mut reply, "remix_response", "remix", Step::RemixResponse)?;
        Ok(self.remix_object.clone())
    }

    fn transform_video_list_request(
        &self,
        query: VideoListQuery<'_>,
        context: VideoRequestContext<'_>,
    ) -> Result<(String, Params), ConfigFailure> {
        let seen = list_query(query, &context);
        self.list_request(seen)
    }

    fn transform_video_list_response(
        &self,
        mut reply: VideoReply<'_>,
    ) -> Result<Params, ConfigFailure> {
        self.record_transform(&mut reply, "list_response", "list", Step::ListResponse)?;
        Ok(self.list_page.clone())
    }

    fn get_error_class(
        &self,
        message: &str,
        status_code: u16,
        headers: ErrorHeaders,
    ) -> ProviderError {
        self.events.push("error_class");
        self.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ErrorCall {
                message: message.to_owned(),
                status_code,
                headers,
            });
        ProviderError::from(ProviderErrorParts {
            status_code,
            message: message.to_owned(),
            headers: None,
            request: None,
            response: None,
            body: None,
            status_code_is_synthesized: false,
        })
    }
}
