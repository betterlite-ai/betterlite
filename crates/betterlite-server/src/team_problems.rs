use crate::list_api::problem::ManagementProblem;
use crate::management_envelopes::ProblemDetail;
mod tests;

#[must_use]
pub fn team_not_found(team_id: &str) -> ManagementProblem {
    ManagementProblem(ProblemDetail {
        r#type: "urn:litellm:error:team-not-found".to_owned(),
        title: "Team not found".to_owned(),
        status: 404,
        detail: format!("Team id={team_id} does not exist in db"),
        allowed: None,
    })
}

#[must_use]
pub fn forbidden(detail: String) -> ManagementProblem {
    ManagementProblem(ProblemDetail {
        r#type: "urn:litellm:error:forbidden".to_owned(),
        title: "Forbidden".to_owned(),
        status: 403,
        detail,
        allowed: None,
    })
}
