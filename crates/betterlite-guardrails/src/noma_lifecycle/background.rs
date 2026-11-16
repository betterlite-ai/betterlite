use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use tokio::runtime::Handle;

use super::hooks::ScanSubject;
use super::{NomaGuardrail, RequestData};

impl NomaGuardrail {
    // LiteLLM's _create_background_noma_check: a missing runtime is logged, never raised
    pub fn create_background_noma_check<F>(&self, check: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        match Handle::try_current() {
            Ok(runtime) => {
                self.tasks.spawn_on(check, &runtime);
            }
            Err(error) => tracing::error!("Failed to create background Noma task: {error}"),
        }
    }

    // The check works on copies, since the request goes on without waiting for it
    pub(super) fn spawn_user_check(&self, request_data: &RequestData, user_auth: &RequestData) {
        let guardrail = self.clone();
        let mut data_copy = request_data.clone();
        let auth_copy = user_auth.clone();
        self.create_background_noma_check(async move {
            guardrail
                .check_user_message_background(&mut data_copy, &auth_copy)
                .await;
        });
    }

    pub(super) fn spawn_response_check(
        &self,
        request_data: &RequestData,
        response: &GuardrailValue,
        user_auth: &RequestData,
    ) {
        let guardrail = self.clone();
        let mut data_copy = request_data.clone();
        let mut response_copy = response.clone();
        let auth_copy = user_auth.clone();
        self.create_background_noma_check(async move {
            guardrail
                .check_llm_response_background(&mut data_copy, &mut response_copy, &auth_copy)
                .await;
        });
    }

    // LiteLLM's _check_user_message_background: a failure is logged and never reaches the request
    pub async fn check_user_message_background(
        &self,
        request_data: &mut RequestData,
        user_auth: &RequestData,
    ) {
        let scanned = self
            .scan(request_data, user_auth, ScanSubject::UserMessage, None)
            .await;
        if let Err(error) = scanned {
            tracing::error!("Noma background user message check failed: {error}");
        }
    }

    // LiteLLM's _check_llm_response_background
    pub async fn check_llm_response_background(
        &self,
        request_data: &mut RequestData,
        response: &mut GuardrailValue,
        user_auth: &RequestData,
    ) {
        let subject = ScanSubject::LlmResponse(response);
        if let Err(error) = self.scan(request_data, user_auth, subject, None).await {
            tracing::error!("Noma background response check failed: {error}");
        }
    }
}
