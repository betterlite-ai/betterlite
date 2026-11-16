use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use tokio::runtime::Handle;
use tokio_util::task::TaskTracker;

use super::hooks::{ScanSubject, scan};
use super::{NomaGuardrail, RequestData};

// What a monitor mode check looks at, copied before the request moves on
#[derive(Clone, Copy)]
pub(super) enum BackgroundSubject<'subject> {
    UserMessage,
    Response(&'subject GuardrailValue),
}

// LiteLLM's _create_background_noma_check: a missing runtime is logged, never raised
pub(super) fn create_background_noma_check<F>(tasks: &TaskTracker, check: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    match Handle::try_current() {
        Ok(runtime) => {
            tasks.spawn_on(check, &runtime);
        }
        Err(error) => tracing::error!("Failed to create background Noma task: {error}"),
    }
}

// The check works on copies, since the request goes on without waiting for it
pub(super) fn spawn_check(
    guardrail: &NomaGuardrail,
    request_data: &RequestData,
    user_auth: &RequestData,
    subject: BackgroundSubject<'_>,
) {
    let tasks = guardrail.tasks.clone();
    let worker = guardrail.clone();
    let mut data_copy = request_data.clone();
    let auth_copy = user_auth.clone();
    match subject {
        BackgroundSubject::UserMessage => create_background_noma_check(&tasks, async move {
            check_background(
                &worker,
                &mut data_copy,
                &auth_copy,
                ScanSubject::UserMessage,
            )
            .await;
        }),
        BackgroundSubject::Response(response) => {
            let mut response_copy = response.clone();
            create_background_noma_check(&tasks, async move {
                let scanned = ScanSubject::LlmResponse(&mut response_copy);
                check_background(&worker, &mut data_copy, &auth_copy, scanned).await;
            });
        }
    }
}

// LiteLLM's _check_user_message_background and _check_llm_response_background: a failure is logged and never reaches the request
pub(super) async fn check_background(
    guardrail: &NomaGuardrail,
    request_data: &mut RequestData,
    user_auth: &RequestData,
    subject: ScanSubject<'_>,
) {
    let failure = match &subject {
        ScanSubject::UserMessage => "Noma background user message check failed",
        ScanSubject::LlmResponse(_) => "Noma background response check failed",
    };
    if let Err(error) = scan(guardrail, request_data, user_auth, subject, None).await {
        tracing::error!("{failure}: {error}");
    }
}
