use aws_sigv4::http_request::SigningError;
use aws_sigv4::sign::v4::signing_params::BuildError;
use http::Error as HttpError;
use http::header::ToStrError;

#[derive(Debug, thiserror::Error)]
pub enum SigV4Error {
    #[error("the signing parameters are invalid")]
    Parameters(#[source] BuildError),
    #[error("the request cannot be signed")]
    Signable(#[source] SigningError),
    #[error("the header {name} cannot be signed because its value is not visible ASCII")]
    HeaderText {
        name: String,
        #[source]
        source: ToStrError,
    },
    #[error("the signed header {name} is not a valid header value")]
    SignedHeader {
        name: String,
        #[source]
        source: HttpError,
    },
}
