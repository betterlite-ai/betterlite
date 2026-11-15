use super::read_call::ReadOperation;
use super::{AgentsApiConfig, Params, PreparedRequest, UpstreamResponse};

pub(crate) struct GetOperation<'name> {
    pub(crate) name: &'name str,
}

impl<Config> ReadOperation<Config> for GetOperation<'_>
where
    Config: AgentsApiConfig,
{
    type Output = Config::GetOutput;

    fn transform_request(
        &self,
        config: &Config,
        api_base: &str,
        litellm_params: &Params,
    ) -> Result<PreparedRequest, Config::Error> {
        config.transform_get_request(self.name, api_base, litellm_params)
    }

    fn transform_response(
        &self,
        config: &Config,
        response: &UpstreamResponse,
    ) -> Result<Self::Output, Config::Error> {
        config.transform_get_response(response, self.name)
    }
}
