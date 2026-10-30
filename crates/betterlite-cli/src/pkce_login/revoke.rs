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
            HttpBody::Form(form(vec![
                ("token", refresh_token),
                ("token_type_hint", "refresh_token"),
                ("client_id", client_id),
            ])),
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
            PkceFailure::new_owned(error_detail(&response)),
        ));
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub enum RevocationUnavailableOrFailure {
    Unavailable(RevocationUnavailable),
    Failure(PkceFailure),
}

fn form(fields: Vec<(&str, &str)>) -> Vec<(String, String)> {
    if fields.is_empty() {
        return Vec::new();
    }
    fields
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}
