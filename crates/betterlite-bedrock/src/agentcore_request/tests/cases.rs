mod arn;
mod policy;
mod signing;
mod transform;
mod url;

use rstest::rstest;

#[rstest]
#[case::runtime_arn_required(arn::runtime_arn_required)]
#[case::arn_region(url::arn_region)]
#[case::runtime_session_id(transform::runtime_session_id)]
#[case::runtime_user_id(transform::runtime_user_id)]
#[case::multimodal_forwarding(transform::multimodal_forwarding)]
#[case::qualifier_query(url::qualifier_query)]
#[case::stream_body_policy(policy::stream_body_policy)]
#[case::agentcore_bearer_signing(signing::agentcore_bearer_signing)]
#[case::agentcore_iam_signing(signing::agentcore_iam_signing)]
fn agentcore_request_cases(#[case] case: fn()) {
    case();
}
