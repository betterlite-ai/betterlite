pub mod chat;
pub mod container_call;
pub mod embedding;
pub mod image_edit;
pub mod image_generation;
pub mod llm_passthrough;
pub mod moderation;
pub mod responses;
pub mod text_completion;
pub mod text_stream;
pub mod video_content;
pub mod video_create_character;
pub mod video_edit;
pub mod video_extension;
pub mod video_generation;
pub mod video_get_character;
pub mod video_list;
pub mod video_remix;
pub mod video_status;

pub mod error;
pub mod log;
pub mod terminal;

// shared fakes and recording ports for every dispatch entry's tests
pub(crate) mod tests;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use betterlite_context::call_context::CallContext;
use betterlite_openai::chat::deployment::ChatDeployment;

use crate::operation::call_types;
use crate::operation::{CallResult, Entry, Operation};

use self::error::DispatchError;
use self::log::CallLog;
use self::terminal::Terminal;
use crate::routes::CallType;
use crate::rules::{RuleDecision, RuleEvaluator, RulePhase, evaluate};

type EntryFuture<'call> =
    Pin<Box<dyn Future<Output = Result<CallResult, DispatchError>> + Send + 'call>>;

pub struct NoRules;

impl RuleEvaluator for NoRules {
    fn evaluate(&self, _index: usize, _input: &str) -> Option<RuleDecision> {
        None
    }
}

pub struct Dispatch {
    deployments: Vec<ChatDeployment>,
    rules: SharedRules,
    log: Arc<dyn CallLog>,
}

// the rule evaluator a caller injects, sized so the rules module's generic call accepts it
struct SharedRules(Arc<dyn RuleEvaluator + Send + Sync>);

impl RuleEvaluator for SharedRules {
    fn evaluate(&self, index: usize, input: &str) -> Option<RuleDecision> {
        self.0.evaluate(index, input)
    }
}

impl Dispatch {
    #[must_use]
    pub fn new<Rules, Log>(
        deployments: Vec<ChatDeployment>,
        rules: Arc<Rules>,
        log: Arc<Log>,
    ) -> Self
    where
        Rules: RuleEvaluator + Send + Sync + 'static,
        Log: CallLog + 'static,
    {
        Self {
            deployments,
            rules: SharedRules(rules),
            log,
        }
    }

    // one arm per entry; the futures are boxed so every entry keeps its own type
    #[must_use]
    #[rustfmt::skip]
    pub fn execute<'call, Metadata, Client>(
        &'call self,
        operation: Operation,
        context: &'call CallContext<'_, Metadata, Client>,
    ) -> EntryFuture<'call>
    where
        Metadata: Sync,
        Client: Sync,
    {
        match operation {
            Operation::Chat(request) => Box::pin(async move { chat::execute(self, request, context).await.map(CallResult::Chat) }),
            Operation::Embedding(request) => Box::pin(async move { embedding::execute(self, request, context).await.map(CallResult::Embedding) }),
            Operation::TextCompletion(request) => Box::pin(async move { text_completion::execute(request, context).await.map(CallResult::TextCompletion) }),
            Operation::TextStream(request) => Box::pin(async move { text_stream::execute(request, context).await.map(CallResult::TextStream) }),
            Operation::Moderation(request) => Box::pin(async move { moderation::execute(request, context).await.map(CallResult::Moderation) }),
            Operation::Responses(request) => Box::pin(async move { responses::execute(request, context).await.map(CallResult::Responses) }),
            Operation::ImageGeneration(request) => Box::pin(async move { image_generation::execute(request, context).await.map(CallResult::ImageGeneration) }),
            Operation::ImageEdit(request) => Box::pin(async move { image_edit::execute(request, context).await.map(CallResult::ImageEdit) }),
            Operation::VideoGeneration(request) => Box::pin(async move { video_generation::execute(request, context).await.map(CallResult::VideoGeneration) }),
            Operation::VideoList(request) => Box::pin(async move { video_list::execute(request, context).await.map(CallResult::VideoList) }),
            Operation::VideoStatus(request) => Box::pin(async move { video_status::execute(request, context).await.map(CallResult::VideoStatus) }),
            Operation::VideoContent(request) => Box::pin(async move { video_content::execute(request, context).await.map(CallResult::VideoContent) }),
            Operation::VideoRemix(request) => Box::pin(async move { video_remix::execute(request, context).await.map(CallResult::VideoRemix) }),
            Operation::VideoCreateCharacter(request) => Box::pin(async move { video_create_character::execute(request, context).await.map(CallResult::VideoCreateCharacter) }),
            Operation::VideoGetCharacter(request) => Box::pin(async move { video_get_character::execute(request, context).await.map(CallResult::VideoGetCharacter) }),
            Operation::VideoEdit(request) => Box::pin(async move { video_edit::execute(request, context).await.map(CallResult::VideoEdit) }),
            Operation::VideoExtension(request) => Box::pin(async move { video_extension::execute(request, context).await.map(CallResult::VideoExtension) }),
            Operation::LlmPassthrough(request) => Box::pin(async move { llm_passthrough::execute(request, context).await.map(CallResult::LlmPassthrough) }),
            Operation::ContainerCall(request) => Box::pin(async move { container_call::execute(request, context).await.map(CallResult::ContainerCall) }),
        }
    }

    // a route type the catalog knows still fails here when no dispatch entry serves it
    pub async fn execute_for<Metadata, Client>(
        &self,
        call_type: CallType,
        operation: Operation,
        context: &CallContext<'_, Metadata, Client>,
    ) -> Result<CallResult, DispatchError>
    where
        Metadata: Sync,
        Client: Sync,
    {
        let Some(entry) = call_types::entry_for(call_type) else {
            return Err(DispatchError::UnknownOperation { call_type });
        };
        let operation_entry = operation.entry();
        if entry != operation_entry {
            return Err(DispatchError::MismatchedOperation {
                call_type,
                entry: <&str>::from(operation_entry),
            });
        }
        self.execute(operation, context).await
    }

    pub(crate) fn deployment(&self, model_id: &str) -> Result<&ChatDeployment, DispatchError> {
        self.deployments
            .iter()
            .find(|deployment| deployment.model_id == model_id)
            .ok_or_else(|| DispatchError::MissingDeployment {
                model_id: model_id.to_owned(),
            })
    }

    // the call's own deadline bounds the deployment timeout, so a bounded call never outlives it
    pub(crate) fn bounded_deployment<Metadata, Client>(
        &self,
        model_id: &str,
        call_id: &str,
        context: &CallContext<'_, Metadata, Client>,
    ) -> Result<ChatDeployment, DispatchError> {
        let deployment = self.deployment(model_id)?;
        let state = context.view().state;
        let Some(deadline) = state.deadline else {
            return Ok(deployment.clone());
        };
        let remaining = deadline
            .duration_since(state.billing_time)
            .map_err(|_error| DispatchError::DeadlineExpired {
                call_id: call_id.to_owned(),
            })?;
        if remaining.is_zero() {
            return Err(DispatchError::DeadlineExpired {
                call_id: call_id.to_owned(),
            });
        }
        let mut bounded = deployment.clone();
        bounded.timeout = bounded.timeout.min(remaining);
        Ok(bounded)
    }

    pub(crate) fn evaluate_rules(
        &self,
        input: Option<&str>,
        model: &str,
    ) -> Result<(), DispatchError> {
        let Some(text) = input else {
            return Ok(());
        };
        evaluate(RulePhase::PreCall, &self.rules, Some(text), model)
            .map_err(|source| DispatchError::Rules { source })
    }

    pub(crate) fn terminal<Metadata, Client>(
        &self,
        entry: Entry,
        call_id: &str,
        context: &CallContext<'_, Metadata, Client>,
    ) -> Terminal {
        let state = context.view().state;
        Terminal {
            log: Arc::clone(&self.log),
            entry,
            call_id: call_id.to_owned(),
            billing: state.billing,
            privacy: state.privacy,
        }
    }
}
