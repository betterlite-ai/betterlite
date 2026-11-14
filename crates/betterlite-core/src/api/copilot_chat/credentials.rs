use std::future::Future;

use betterlite_openai::copilot::api_key::clock::Clock;
use betterlite_openai::copilot::api_key::error::ApiKeyError;
use betterlite_openai::copilot::api_key::transport::ApiKeyTransport;
use betterlite_openai::copilot::device_auth::Authenticator;
use betterlite_openai::copilot::device_auth::files::FileAccess;
use betterlite_openai::copilot::device_auth::output::OperatorOutput;
use betterlite_openai::copilot::device_auth::sleeper::Sleeper;
use betterlite_openai::copilot::device_auth::transport::Transport;
use secrecy::SecretString;

pub trait CopilotCredentials {
    fn api_base(&mut self) -> impl Future<Output = Option<String>> + Send;
    fn api_key(&mut self)
    -> impl Future<Output = Result<Option<SecretString>, ApiKeyError>> + Send;
}

impl<F, C, T, O> CopilotCredentials for Authenticator<F, C, T, O>
where
    F: FileAccess + Send,
    C: Clock + Sleeper + Send,
    T: ApiKeyTransport + Transport + Send,
    O: OperatorOutput + Send,
{
    fn api_base(&mut self) -> impl Future<Output = Option<String>> + Send {
        Self::get_api_base(self)
    }

    fn api_key(
        &mut self,
    ) -> impl Future<Output = Result<Option<SecretString>, ApiKeyError>> + Send {
        Self::get_api_key(self)
    }
}
