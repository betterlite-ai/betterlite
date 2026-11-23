#![cfg(test)]

use std::future::Future;

use rstest::rstest;
use tokio::task::JoinSet;

use super::{ExchangeSubjectToken, RefusalCode, SubjectIdentity, SubjectTokenRefusal};

struct FakeContext {
    expected_token: String,
    answer: Result<SubjectIdentity, SubjectTokenRefusal>,
}

struct FakeExchanger;

impl ExchangeSubjectToken<FakeContext> for FakeExchanger {
    fn exchange(
        &self,
        subject_token: &str,
        context: &FakeContext,
    ) -> impl Future<Output = Result<SubjectIdentity, SubjectTokenRefusal>> + Send {
        let answer = if subject_token == context.expected_token {
            context.answer.clone()
        } else {
            Err(SubjectTokenRefusal {
                error: RefusalCode::InvalidRequest,
                description: "unexpected subject token".to_owned(),
            })
        };
        async move { answer }
    }
}

async fn exchange_with<Context, Exchanger>(
    exchanger: &Exchanger,
    subject_token: &str,
    context: &Context,
) -> Result<SubjectIdentity, SubjectTokenRefusal>
where
    Exchanger: ExchangeSubjectToken<Context>,
{
    exchanger.exchange(subject_token, context).await
}

fn team_identity() -> SubjectIdentity {
    SubjectIdentity {
        user_id: "user-1".to_owned(),
        team_id: Some("team-1".to_owned()),
    }
}

fn refused() -> SubjectTokenRefusal {
    SubjectTokenRefusal {
        error: RefusalCode::InvalidRequest,
        description: "the identity provider rejected the token".to_owned(),
    }
}

#[rstest]
#[case::unsupported_grant_type(RefusalCode::UnsupportedGrantType, "unsupported_grant_type")]
#[case::invalid_request(RefusalCode::InvalidRequest, "invalid_request")]
#[case::temporarily_unavailable(RefusalCode::TemporarilyUnavailable, "temporarily_unavailable")]
fn refusal_codes_render_their_oauth_text(#[case] code: RefusalCode, #[case] text: &str) {
    assert_eq!(code.as_str(), text);
    assert_eq!(code.to_string(), text);
}

#[rstest]
#[case::identity_with_team(Ok(team_identity()))]
#[case::identity_without_team(Ok(SubjectIdentity {
    user_id: "user-2".to_owned(),
    team_id: None,
}))]
#[case::refusal(Err(refused()))]
#[tokio::test]
async fn a_generic_port_runner_returns_what_the_exchanger_answers(
    #[case] answer: Result<SubjectIdentity, SubjectTokenRefusal>,
) {
    let context = FakeContext {
        expected_token: "idp-token".to_owned(),
        answer: answer.clone(),
    };
    let exchanged = exchange_with(&FakeExchanger, "idp-token", &context).await;
    assert_eq!(exchanged, answer);
}

#[tokio::test]
async fn the_subject_token_reaches_the_implementation_unchanged() {
    let context = FakeContext {
        expected_token: "idp-token".to_owned(),
        answer: Ok(team_identity()),
    };
    let exchanged = exchange_with(&FakeExchanger, "a-different-token", &context).await;
    assert_eq!(
        exchanged,
        Err(SubjectTokenRefusal {
            error: RefusalCode::InvalidRequest,
            description: "unexpected subject token".to_owned(),
        })
    );
}

#[tokio::test]
async fn the_exchange_future_runs_as_a_spawned_task() {
    let identity = team_identity();
    let context = FakeContext {
        expected_token: "idp-token".to_owned(),
        answer: Ok(identity.clone()),
    };
    let exchanger = FakeExchanger;
    let mut tasks = JoinSet::new();
    tasks.spawn(async move { exchanger.exchange("idp-token", &context).await });
    let joined = tasks.join_next().await.unwrap().unwrap();
    assert_eq!(joined.unwrap(), identity);
}
