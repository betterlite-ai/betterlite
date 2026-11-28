use std::time::SystemTime;

use aws_credential_types::Credentials;
use betterlite_sigv4::credentials::Credentials as SigV4Credentials;
use betterlite_sigv4::sign::header_map::sign_in_place;
use bytes::Bytes;
use http::{Request, header};

use super::error::McpSigV4Error;

pub(super) fn sign_request(
    request: &mut Request<Bytes>,
    credentials: &Credentials,
    region_name: &str,
    service_name: &str,
    now: SystemTime,
) -> Result<(), McpSigV4Error> {
    if request.headers().contains_key(header::DATE) {
        return Err(McpSigV4Error::DateHeader);
    }
    let sigv4_credentials = SigV4Credentials::from(credentials);
    sign_in_place(request, &sigv4_credentials, service_name, region_name, now)
        .map_err(|source| McpSigV4Error::Signing { source })
}
