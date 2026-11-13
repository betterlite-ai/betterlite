use super::fixture::{Answer, assume_role_answer, denied_identity, web_identity_answer};
use super::input::{Case, ReplyCredentials, WebIdentityReply};
use super::ops::{FixtureError, failure, fixture, irsa_json, tags_of};
use crate::helpers::{HelperError, HelperResult};
use betterlite_bedrock::web_identity::irsa::{CrossAccountIrsa, handle_irsa_cross_account};
use std::collections::BTreeMap;
fn answers(reply: &ReplyCredentials, irsa_reply: &WebIdentityReply) -> BTreeMap<String, Answer> {
    BTreeMap::from([
        (
            "AssumeRoleWithWebIdentity".to_owned(),
            web_identity_answer(irsa_reply),
        ),
        ("GetCallerIdentity".to_owned(), denied_identity()),
        ("AssumeRole".to_owned(), assume_role_answer(reply)),
    ])
}
pub(super) fn cross_account_irsa(case: &Case) -> HelperResult {
    let (Some(token_file), Some(irsa_reply), Some(WebIdentityReply::Credentials(reply))) = (
        case.token_file.as_ref(),
        case.irsa_reply.as_ref(),
        case.reply.as_ref(),
    ) else {
        return Err(Box::<HelperError>::from(FixtureError::Case(
            "the cross-account case needs a token file and two credentials replies",
        )));
    };
    let files = [(token_file.path.as_str(), token_file.content.as_str())];
    let (native, record, kwargs, runtime) = fixture(case, &files, answers(reply, irsa_reply))?;
    let tags = tags_of(&case.tags);
    let input = CrossAccountIrsa {
        irsa_role_arn: &case.irsa_role_arn,
        role_arn: &case.role_arn,
        session_name: &case.session_name,
        token_file: &token_file.path,
        external_id: case.external_id.as_deref(),
        tags: Some(tags.as_slice()),
    };
    let response = runtime
        .block_on(handle_irsa_cross_account(&input, &kwargs, &native))
        .map_err(|error| failure(&error))?;
    Ok(irsa_json(&record, &response))
}
