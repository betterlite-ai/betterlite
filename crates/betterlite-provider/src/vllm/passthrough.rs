use betterlite_config::loader::EnvironmentLookup;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::passthrough::transformation::BasePassthroughConfig;
use betterlite_openai::passthrough::transformation::types::PassthroughUrlRequest;
use url::Url;

use super::error::VllmError;
use super::model_info::VllmModelInfo;

pub struct VllmPassthroughConfig<'environment> {
    pub environment: &'environment (dyn EnvironmentLookup + Sync),
}

impl BasePassthroughConfig for VllmPassthroughConfig<'_> {
    type Error = VllmError;
    type Logged = Object;

    fn is_streaming_request(&self, _endpoint: &str, request_data: &Object) -> bool {
        request_data.contains_key("stream")
    }

    fn get_complete_url(
        &self,
        request: &PassthroughUrlRequest<'_>,
    ) -> Result<(Url, String), Self::Error> {
        let info = VllmModelInfo {
            environment: self.environment,
        };
        let base = info.get_api_base(request.api_base)?;
        let url = self.format_url(request.target.endpoint, &base, request.request_query_params)?;
        Ok((url, base))
    }
}
