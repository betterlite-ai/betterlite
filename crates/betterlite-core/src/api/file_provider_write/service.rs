use betterlite_models::resolution::{ResolutionContext, ResolutionRequest, resolve};
use betterlite_openai::files::records::{FileCreate, FileDeleted, FileObject};
use chrono::{DateTime, FixedOffset};
use uuid::Uuid;

use crate::api::file_http_upload::service::Mode;
use crate::api::resource_sdk_settings::ResourceSdkSettings;
use crate::api::resource_sdk_settings::timeout::FineTuningTimeout;

use super::arguments::{
    create_request, delete_params, handler, passthrough_guard, provider_id, request_options,
    upload_provider, write_timeout,
};
use super::{
    ConfigDelete, ConfigUpload, DeleteInput, DeleteLogger, DeleteTarget, FileWriteError,
    FileWriteProviders, LoggerSeed, ProviderCall, UploadInput, UploadTarget, WriteParams,
};

type Written<Value, Providers> =
    Result<Value, FileWriteError<<Providers as FileWriteProviders>::Error>>;

// OPENAI_COMPATIBLE_BATCH_AND_FILES_PROVIDERS, the names the OpenAI files client serves
const OPENAI_COMPATIBLE: [&str; 3] = ["openai", "hosted_vllm", "litellm_proxy"];

pub struct FileWrite<'deps, Providers> {
    pub providers: Providers,
    pub settings: ResourceSdkSettings,
    pub resolution: &'deps ResolutionContext<'deps>,
    pub clock: fn() -> DateTime<FixedOffset>,
    pub call_ids: fn() -> Uuid,
}

// the two values every provider branch receives beside its credentials
#[derive(Debug, Clone, Copy)]
struct Invocation {
    mode: Mode,
    timeout: FineTuningTimeout,
}

impl<Providers: FileWriteProviders> FileWrite<'_, Providers> {
    // create_file, with acreate_file as its asynchronous mode: the guard, the deadline, then config-first dispatch
    pub async fn create_file(
        &self,
        mode: Mode,
        input: &UploadInput<Providers::Logger, Providers::Client>,
    ) -> Written<FileObject, Providers> {
        let params = &input.params;
        let Some(logger) = params.logger.as_ref() else {
            return Err(FileWriteError::LoggingObjectRequired);
        };
        let provider = upload_provider(mode, input);
        let named = provider.as_deref();
        passthrough_guard::<Providers::Error>(params.passthrough, named, input.purpose)?;
        let timeout = write_timeout::<_, _, Providers::Error>(params, named.unwrap_or("None"))?;
        let invocation = Invocation { mode, timeout };
        let data = create_request(input);
        let config = self
            .providers
            .files_config(provider_id::<Providers::Error>(named)?);
        let target = match config {
            Some(config) => UploadTarget::Config(ConfigUpload {
                config,
                mode,
                headers: data.extra_headers.clone(),
                data,
                api_base: params.api_base.as_deref(),
                api_key: params.api_key.as_ref(),
                params,
                logger,
                client: handler(params),
                timeout,
            }),
            None => self.sdk_upload(invocation, named.unwrap_or("None"), params, data)?,
        };
        self.providers
            .upload(target)
            .await
            .map_err(FileWriteError::Provider)
    }

    fn sdk_upload<'call>(
        &self,
        invocation: Invocation,
        provider: &str,
        params: &'call WriteParams<Providers::Logger, Providers::Client>,
        data: FileCreate,
    ) -> Written<UploadTarget<'call, Providers>, Providers> {
        if OPENAI_COMPATIBLE.contains(&provider) {
            return Ok(UploadTarget::OpenAi {
                call: self.openai_call(invocation, params),
                data,
            });
        }
        if provider == "azure" {
            return Ok(UploadTarget::Azure {
                call: self.azure_call(invocation, params),
                data,
                params,
            });
        }
        Err(FileWriteError::UnsupportedUpload {
            provider: provider.to_owned(),
        })
    }

    // file_delete, with afile_delete as its asynchronous mode: the model hint, then compatible-first dispatch
    pub async fn delete_file(
        &self,
        mode: Mode,
        input: &DeleteInput<Providers::Logger, Providers::Client>,
    ) -> Written<FileDeleted, Providers> {
        let params = &input.params;
        let provider = self.hinted_provider(input);
        let timeout = write_timeout::<_, _, Providers::Error>(params, &provider)?;
        let invocation = Invocation { mode, timeout };
        let file_id = input.file_id.as_str();
        let target = if OPENAI_COMPATIBLE.contains(&provider.as_str()) {
            DeleteTarget::OpenAi {
                call: self.openai_call(invocation, params),
                file_id,
            }
        } else if provider == "azure" {
            DeleteTarget::Azure {
                call: self.azure_call(invocation, params),
                file_id,
                client: params.client.as_ref(),
                params: delete_params(params),
            }
        } else {
            self.config_delete(invocation, provider, input)?
        };
        self.providers
            .delete(target)
            .await
            .map_err(FileWriteError::Provider)
    }

    fn config_delete<'call>(
        &self,
        invocation: Invocation,
        provider: String,
        input: &'call DeleteInput<Providers::Logger, Providers::Client>,
    ) -> Written<DeleteTarget<'call, Providers>, Providers> {
        let id = provider_id::<Providers::Error>(Some(provider.as_str()))?;
        let Some(config) = self.providers.files_config(id) else {
            return Err(FileWriteError::UnsupportedDelete { provider });
        };
        let params = &input.params;
        Ok(DeleteTarget::Config(ConfigDelete {
            config,
            mode: invocation.mode,
            file_id: &input.file_id,
            params: delete_params(params),
            headers: input.extra_headers.clone().unwrap_or_default(),
            logger: self.delete_logger(invocation.mode, params),
            client: handler(params),
            timeout: invocation.timeout,
        }))
    }

    // get_llm_provider(model, custom_llm_provider) may rename the provider; LiteLLM swallows its failure
    fn hinted_provider(&self, input: &DeleteInput<Providers::Logger, Providers::Client>) -> String {
        let declared = input
            .custom_llm_provider
            .clone()
            .unwrap_or_else(|| "openai".to_owned());
        let Some(model) = input.model.as_deref() else {
            return declared;
        };
        let request = ResolutionRequest {
            model,
            custom_llm_provider: Some(declared.as_str()),
            api_base: None,
            api_key: None,
        };
        let resolved = resolve(&request, self.resolution);
        resolved.map_or(declared, |found| found.provider.slug().to_owned())
    }

    fn delete_logger<'call>(
        &self,
        mode: Mode,
        params: &'call WriteParams<Providers::Logger, Providers::Client>,
    ) -> DeleteLogger<'call, Providers::Logger> {
        if let Some(existing) = params.logger.as_ref() {
            return DeleteLogger::Existing(existing);
        }
        DeleteLogger::Created(LoggerSeed {
            call_type: match mode {
                Mode::Sync => "file_delete",
                Mode::Asynchronous => "afile_delete",
            },
            start_time: (self.clock)(),
            litellm_call_id: params
                .litellm_call_id
                .clone()
                .unwrap_or_else(|| Some((self.call_ids)().to_string())),
            function_id: params.id.clone().unwrap_or_default(),
        })
    }

    // get_openai_credentials: the call's value, then the litellm settings, then the environment
    fn openai_call<Logger, Client>(
        &self,
        invocation: Invocation,
        params: &WriteParams<Logger, Client>,
    ) -> ProviderCall {
        let options = request_options(params);
        ProviderCall {
            mode: invocation.mode,
            api_base: Some(self.settings.openai_base(&options)),
            api_key: self.settings.openai_api_key(&options).cloned(),
            api_version: None,
            organization: self
                .settings
                .openai_organization(&options)
                .map(str::to_owned),
            timeout: invocation.timeout,
            max_retries: params.max_retries,
        }
    }

    // get_azure_credentials: the same chain over the azure names
    fn azure_call<Logger, Client>(
        &self,
        invocation: Invocation,
        params: &WriteParams<Logger, Client>,
    ) -> ProviderCall {
        let options = request_options(params);
        ProviderCall {
            mode: invocation.mode,
            api_base: self.settings.azure_base(&options).map(str::to_owned),
            api_key: self.settings.azure_api_key(&options).cloned(),
            api_version: self.settings.azure_version(&options).map(str::to_owned),
            organization: None,
            timeout: invocation.timeout,
            max_retries: params.max_retries,
        }
    }
}
