use super::read_call::ReadOperation;
use super::{AgentsApiConfig, Params, PreparedRequest, UpstreamResponse};

pub(crate) struct ListOperation;

impl<Config> ReadOperation<Config> for ListOperation
where
    Config: AgentsApiConfig,
{
    type Output = Config::ListOutput;

    fn input(&self) -> &str {
        "list_agents"
    }

    fn transform_request(
        &self,
        config: &Config,
        api_base: &str,
        litellm_params: &Params,
    ) -> Result<PreparedRequest, Config::Error> {
        config.transform_list_request(api_base, litellm_params)
    }

    fn transform_response(
        &self,
        config: &Config,
        response: &UpstreamResponse,
    ) -> Result<Self::Output, Config::Error> {
        config.transform_list_response(response)
    }
}
