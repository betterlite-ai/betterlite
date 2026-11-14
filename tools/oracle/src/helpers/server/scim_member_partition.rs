use std::collections::BTreeSet;
use std::fmt::Display;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_server::scim_member_identity::classified::{
    ClassifiedGroupMember, PartitionedMembers, admitted_member_ids, partition_classified_members,
    roster_entries_named_by,
};
use betterlite_server::scim_member_identity::error::ScimMemberIdentityError;
use betterlite_server::scim_member_identity::member_value::{
    json_object_fields, member_value, metadata_has_scim_provenance, normalized_member_type,
};
use betterlite_server::scim_records::group::{Member, MemberKind};
use serde::{Deserialize, Serialize};

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct MemberInput {
    value: String,
    #[serde(rename = "type")]
    kind: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct RemovalInput {
    value: String,
    roster: Vec<String>,
    resolved: Vec<String>,
}

#[derive(Deserialize)]
pub(crate) struct Input {
    members: Vec<MemberInput>,
    metadata: Vec<JsonData>,
    entries: Vec<ClassifiedGroupMember>,
    created_ids: Vec<String>,
    removals: Vec<RemovalInput>,
}

#[derive(Serialize)]
struct Answers {
    members: Vec<MemberAnswer>,
    metadata: Vec<MetadataAnswer>,
    partition: PartitionedMembers,
    admitted: Vec<String>,
    roster: Vec<RosterAnswer>,
}

#[derive(Serialize)]
struct MemberAnswer {
    value: String,
    #[serde(rename = "type")]
    kind: Option<String>,
}

#[derive(Serialize)]
struct MetadataAnswer {
    is_object: bool,
    provenance: bool,
}

#[derive(Serialize)]
struct RosterAnswer {
    value: String,
    entries: Vec<String>,
}

fn member_answers(members: &[MemberInput]) -> Result<Vec<MemberAnswer>, ScimMemberIdentityError> {
    let mut answers = Vec::new();
    for input in members {
        let member = Member {
            value: input.value.clone(),
            display: None,
            kind: MemberKind(input.kind.clone()),
        };
        answers.push(MemberAnswer {
            value: member_value(&member)?.to_owned(),
            kind: normalized_member_type(&member),
        });
    }
    Ok(answers)
}

fn metadata_answer(raw: &JsonData) -> MetadataAnswer {
    let fields = json_object_fields(raw);
    MetadataAnswer {
        is_object: fields.is_some(),
        provenance: fields.is_some_and(metadata_has_scim_provenance),
    }
}

fn roster_answers(removals: &[RemovalInput]) -> Vec<RosterAnswer> {
    let mut answers = Vec::new();
    for removal in removals {
        let roster: BTreeSet<String> = removal.roster.iter().cloned().collect();
        answers.push(RosterAnswer {
            value: removal.value.clone(),
            entries: roster_entries_named_by(&removal.value, &roster, &removal.resolved),
        });
    }
    answers
}

fn answers(input: &Input) -> Result<Answers, ScimMemberIdentityError> {
    let members = member_answers(&input.members)?;
    let metadata = input.metadata.iter().map(metadata_answer).collect();
    let created: BTreeSet<String> = input.created_ids.iter().cloned().collect();
    let admitted = admitted_member_ids(&input.entries, &created);
    Ok(Answers {
        members,
        metadata,
        partition: partition_classified_members(&input.entries),
        admitted,
        roster: roster_answers(&input.removals),
    })
}

fn member_failure(error: &ScimMemberIdentityError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "HTTPException".to_owned(),
        status: Some(error.status_code().as_u16()),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn failure<Error>(error: Error) -> Box<HelperError>
where
    Error: Display,
{
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let built = answers(input).map_err(|error| member_failure(&error))?;
        serde_json::to_value(built).map_err(failure)
    }
}
