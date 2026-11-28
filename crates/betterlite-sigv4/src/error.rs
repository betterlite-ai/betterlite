use aws_sigv4::http_request::SigningError;
use aws_sigv4::sign::v4::signing_params::BuildError;

#[derive(Debug, thiserror::Error)]
pub enum SigV4Error {
    #[error("the signing parameters are invalid")]
    Parameters(#[source] BuildError),
    #[error("the request cannot be signed")]
    Signable(#[source] SigningError),
}
