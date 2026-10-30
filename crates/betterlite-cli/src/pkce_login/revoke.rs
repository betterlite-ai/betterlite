use super::{
    HTTP_TIMEOUT_SECONDS, Http, HttpBody,
    models::{PkceFailure, RevocationUnavailable},
    wire::{error_detail, refuse_redirect},
};

pub fn revoke_credential<H>(
    contract: &super::models::CliAuthContract,
    client_id: &str,
    refresh_token: &str,
    http: &H,
) -> Result<(), RevocationUnavailableOrFailure>
where
    H: Http,
{
    let response = http
        .post(
            &contract.revocation_endpoint,
            HttpBody::Form(vec![
                (String::from("token"), String::from(refresh_token)),
                (
                    String::from("token_type_hint"),
                    String::from("refresh_token"),
                ),
                (String::from("client_id"), String::from(client_id)),
            ]),
            HTTP_TIMEOUT_SECONDS,
            false,
        )
        .map_err(RevocationUnavailableOrFailure::Failure)?;
    refuse_redirect("revocation", &response).map_err(RevocationUnavailableOrFailure::Failure)?;
    if response.status == 503 {
        return Err(RevocationUnavailableOrFailure::Unavailable(
            RevocationUnavailable {
                reason: error_detail(&response),
            },
        ));
    }
    if response.status != 200 {
        return Err(RevocationUnavailableOrFailure::Failure(
            PkceFailure::Protocol(error_detail(&response)),
        ));
    }
    Ok(())
}

#[derive(Debug)]
pub enum RevocationUnavailableOrFailure {
    Unavailable(RevocationUnavailable),
    Failure(PkceFailure),
}
