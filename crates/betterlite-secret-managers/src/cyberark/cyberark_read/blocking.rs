use super::secret_url;
use crate::cyberark::CyberArkSecretManager;
use crate::cyberark::error::CyberArkError;
use crate::cyberark::transport::{ConjurRequest, ConjurResponse, ConjurTransport, HttpMethod};

pub(crate) fn fetch<T>(
    manager: &CyberArkSecretManager<T>,
    secret_name: &str,
) -> Result<ConjurResponse, CyberArkError<T::Error>>
where
    T: ConjurTransport,
{
    let header = manager.authorization_blocking()?;
    let request = ConjurRequest {
        method: HttpMethod::Get,
        url: secret_url(manager, secret_name),
        authorization: Some(&header),
        content_type: None,
        body: None,
        verify_tls: manager.settings.verify_tls,
        client_certificate: None,
    };
    let reply = manager.transport.send_blocking(request);
    reply.map_err(CyberArkError::Transport)?.require_success()
}
