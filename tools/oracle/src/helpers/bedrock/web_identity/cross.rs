use std::collections::BTreeMap;

use betterlite_bedrock::web_identity::irsa::{CrossAccountIrsa, handle_irsa_cross_account};

use super::answers::{assume_role_answer, credentials_xml, denied_identity};
use super::fixture::Answer;
use super::input::CrossAccountCase;
use super::ops::{failure, fixture_failure, fixture_of, irsa_json, tags_of};
use super::ports;
use crate::helpers::HelperResult;

fn answers(case: &CrossAccountCase) -> BTreeMap<String, Answer> {
    BTreeMap::from([
        (
            "AssumeRoleWithWebIdentity".to_owned(),
            Answer {
                status: 200,
                body: format!(
                    "<AssumeRoleWithWebIdentityResponse><AssumeRoleWithWebIdentityResult>{}</AssumeRoleWithWebIdentityResult></AssumeRoleWithWebIdentityResponse>",
                    credentials_xml(&case.irsa_reply)
                ),
            },
        ),
        ("GetCallerIdentity".to_owned(), denied_identity()),
        ("AssumeRole".to_owned(), assume_role_answer(&case.reply)),
    ])
}

pub(super) fn cross_account_irsa(case: &CrossAccountCase) -> HelperResult {
    let kwargs = super::fixture::kwargs(
        &case.environment,
        case.sts_endpoint.as_deref(),
        case.region_name.as_deref(),
        case.verify,
    )
    .map_err(fixture_failure)?;
    let files = [(
        case.token_file.path.clone(),
        case.token_file.content.as_bytes().to_vec(),
    )];
    let (native, record) = fixture_of(&case.environment, &files, answers(case));
    let tags = tags_of(&case.tags);
    let input = CrossAccountIrsa {
        irsa_role_arn: &case.irsa_role_arn,
        role_arn: &case.role_arn,
        session_name: &case.session_name,
        token_file: &case.token_file.path,
        external_id: case.external_id.as_deref(),
        tags: Some(tags.as_slice()),
    };
    let runtime = ports::runtime().map_err(fixture_failure)?;
    let response = runtime
        .block_on(handle_irsa_cross_account(&input, &kwargs, &native))
        .map_err(|error| failure(&error))?;
    Ok(irsa_json(&record, &response))
}
