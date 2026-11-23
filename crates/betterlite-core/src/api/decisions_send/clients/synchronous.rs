use betterlite_http::interactions::base::clients::HttpClients;
use betterlite_http::interactions::base::{ClientError, Params, RequestFailure};

use super::{Answer, DecisionsRequest, DecisionsSyncTransport, finish_blocking};

// decisions(): _get_httpx_client() with no litellm_params, then one blocking POST
impl DecisionsSyncTransport for HttpClients {
    fn send_sync(&mut self, request: DecisionsRequest) -> Result<Answer, ClientError> {
        let client = self.sync_client(&Params::new(), None)?;
        let mut builder = client.post(request.url).headers(request.headers);
        if let Some(deadline) = request.timeout {
            builder = builder.timeout(deadline);
        }
        let answer = match builder.body(request.body).send() {
            Ok(sent) => {
                let status = sent.status().as_u16();
                let headers = sent.headers().clone();
                finish_blocking(sent, status, headers)
            }
            Err(error) => Err(RequestFailure::Transport(error)),
        };
        Ok(answer)
    }
}
