use super::connection;
use super::{
    AzureCreateCall, CreateFineTuningJobCall, FineTuningCreateDispatch, FineTuningCreateError,
    FineTuningCreateTarget, OpenAiCreateCall, VertexCreateCall,
};
use crate::api::resource_sdk_settings::timeout::FineTuningTimeout;
use crate::api::resource_sdk_settings::{ResourceSdkRequestOptions, ResourceSdkSettings};
use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::fine_tuning::job::{FineTuneCreate, FineTuneJob};
use serde_json::{Number, from_value, to_value};
use std::error::Error;
pub struct FineTuningCreate<Dispatch> {
    pub dispatch: Dispatch,
    pub settings: ResourceSdkSettings,
}
impl<Dispatch> FineTuningCreate<Dispatch>
where
    Dispatch: FineTuningCreateDispatch,
    Dispatch::Error: Error + Send + Sync + 'static,
{
    pub async fn create(
        &self,
        call: &CreateFineTuningJobCall,
    ) -> Result<FineTuneJob, FineTuningCreateError<Dispatch::Error>> {
        if call.is_async {
            tracing::debug!(
                "inside acreate_fine_tuning_job model={} and kwargs={:?}",
                call.model,
                call.params
            );
        }
        let outcome = async {
            let timeout = connection::resolve_timeout(call)?;
            let hyperparameters = call.hyperparameters.clone();
            let target = self.target(call, timeout, hyperparameters)?;
            self.dispatch
                .create(target)
                .await
                .map_err(FineTuningCreateError::Dispatch)
        }
        .await;
        if let Err(error) = &outcome {
            tracing::error!("got exception in create_fine_tuning_job={error}");
        }
        outcome
    }
    // create_fine_tuning_job branches on the provider's wire name; only the three declared names resolve
    fn target(
        &self,
        call: &CreateFineTuningJobCall,
        timeout: FineTuningTimeout,
        hyperparameters: Extra,
    ) -> Result<FineTuningCreateTarget, FineTuningCreateError<Dispatch::Error>> {
        let options = ResourceSdkRequestOptions::from(&call.params);
        match call.custom_llm_provider.as_str() {
            "openai" => Ok(FineTuningCreateTarget::OpenAi(OpenAiCreateCall {
                is_async: call.is_async,
                data: job_data(call, &hyperparameters, None)?,
                api_base: self.settings.openai_base(&options),
                api_key: self.settings.openai_api_key(&options).cloned(),
                api_version: self
                    .settings
                    .openai_api_version(&options)
                    .map(str::to_owned),
                organization: self
                    .settings
                    .openai_organization(&options)
                    .map(str::to_owned),
                timeout,
                max_retries: call.params.max_retries,
            })),
            "azure" => self.azure(call, timeout, &options),
            "vertex_ai" => Ok(FineTuningCreateTarget::Vertex(VertexCreateCall {
                is_async: call.is_async,
                data: job_data(call, &hyperparameters, None)?,
                original_hyperparameters: hyperparameters,
                api_base: call.params.api_base.clone().unwrap_or_default(),
                vertex_project: self.settings.vertex_project(&options).map(str::to_owned),
                vertex_location: self.settings.vertex_location(&options).map(str::to_owned),
                vertex_credentials: self.settings.vertex_credentials(&options).cloned(),
                timeout,
            })),
            provider => Err(FineTuningCreateError::Unsupported {
                provider: provider.to_owned(),
            }),
        }
    }
    // the public extra_body argument is discarded: the helper starts from the params one, which is empty
    fn azure(
        &self,
        call: &CreateFineTuningJobCall,
        timeout: FineTuningTimeout,
        options: &ResourceSdkRequestOptions,
    ) -> Result<FineTuningCreateTarget, FineTuningCreateError<Dispatch::Error>> {
        let (hyperparameters, azure_hyperparameters) = split_hyperparameters(call);
        let mut extra_body = Extra::new();
        prepare_azure_extra_body(
            &mut extra_body,
            call.params.training_type.as_ref(),
            &azure_hyperparameters,
        );
        let api_base = self.settings.azure_base(options).map(str::to_owned);
        let api_key = self.settings.azure_api_key(options).cloned();
        let api_version = self.settings.azure_version(options).map(str::to_owned);
        let organization = self.settings.azure_organization(options).map(str::to_owned);
        let data = job_data(
            call,
            &hyperparameters,
            (!extra_body.is_empty()).then_some(&extra_body),
        )?;
        Ok(FineTuningCreateTarget::Azure(AzureCreateCall {
            is_async: call.is_async,
            data,
            api_base,
            api_key,
            api_version,
            organization,
            timeout,
            max_retries: call.params.max_retries,
        }))
    }
}
// Azure pops prompt_loss_weight from the caller's map before the OpenAI-spec hyperparameters are built
fn split_hyperparameters(call: &CreateFineTuningJobCall) -> (Extra, Extra) {
    let mut hyperparameters = call.hyperparameters.clone();
    let mut azure = Extra::new();
    if call.custom_llm_provider == "azure"
        && let Some(value) = hyperparameters.shift_remove("prompt_loss_weight")
    {
        azure.insert("prompt_loss_weight".to_owned(), value);
    }
    (hyperparameters, azure)
}
// kwargs.trainingType is copied first, then the extracted Azure hyperparameters overwrite a collision
pub fn prepare_azure_extra_body(
    extra_body: &mut Extra,
    training_type: Option<&Json>,
    azure_specific_hyperparameters: &Extra,
) {
    if let Some(value) = training_type {
        extra_body.insert("trainingType".to_owned(), value.clone());
    }
    extra_body.extend(azure_specific_hyperparameters.clone());
}
// _build_fine_tuning_job_data().model_dump(exclude_none=True): unset fields and null hyperparameters are left out of the wire body
fn job_data<DispatchError>(
    call: &CreateFineTuningJobCall,
    hyperparameters: &Extra,
    extra_body: Option<&Extra>,
) -> Result<FineTuneCreate, FineTuningCreateError<DispatchError>>
where
    DispatchError: Error + 'static,
{
    let mut fields = Extra::new();
    fields.insert("model".to_owned(), Json::Text(call.model.clone()));
    fields.insert(
        "training_file".to_owned(),
        Json::Text(call.training_file.clone()),
    );
    let mut retained = hyperparameters.clone();
    retained.retain(|_, value| !matches!(value, Json::Null));
    fields.insert("hyperparameters".to_owned(), Json::Object(retained));
    if let Some(seed) = call.seed {
        fields.insert("seed".to_owned(), Json::Number(Number::from(seed)));
    }
    if let Some(suffix) = &call.suffix {
        fields.insert("suffix".to_owned(), Json::Text(suffix.clone()));
    }
    if let Some(validation_file) = &call.validation_file {
        fields.insert(
            "validation_file".to_owned(),
            Json::Text(validation_file.clone()),
        );
    }
    if let Some(integrations) = &call.integrations {
        fields.insert("integrations".to_owned(), Json::List(integrations.clone()));
    }
    if let Some(body) = extra_body {
        fields.insert("extra_body".to_owned(), Json::Object(body.clone()));
    }
    let encoded = to_value(fields).map_err(FineTuningCreateError::Data)?;
    from_value(encoded).map_err(FineTuningCreateError::Data)
}
