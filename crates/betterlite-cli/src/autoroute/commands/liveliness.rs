use crate::autoroute::process::liveliness::LivelinessProbe;
use crate::pkce_login::Http;

// LiteLLM's requests.get uses a two-second timeout, so one hung probe cannot eat the readiness window
pub const LIVELINESS_TIMEOUT_SECONDS: u64 = 2;

pub struct HttpLiveliness<'http> {
    pub http: &'http dyn Http,
}

impl LivelinessProbe for HttpLiveliness<'_> {
    fn is_live(&self, url: &str) -> bool {
        self.http
            .get(url, LIVELINESS_TIMEOUT_SECONDS)
            .is_ok_and(|response| response.status == 200)
    }
}
