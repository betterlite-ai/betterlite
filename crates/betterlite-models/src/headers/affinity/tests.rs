#![cfg(test)]

use indexmap::IndexMap;
use rstest::rstest;

use crate::headers::affinity::{
    AffinityError, AffinityParams, SessionMetadata, add_provider_affinity_header,
    get_stable_session_id, validate_affinity_header,
};

struct MetaSpec {
    session: Option<&'static str>,
    generated: Option<bool>,
}

struct ParamsSpec {
    header: Option<&'static str>,
    session: Option<&'static str>,
    litellm_session: Option<&'static str>,
    metadata: Option<MetaSpec>,
    litellm_metadata: Option<MetaSpec>,
}

enum Scenario {
    Add {
        headers: &'static [(&'static str, &'static str)],
        params: ParamsSpec,
    },
    Session(ParamsSpec),
    Validate(&'static str),
}

enum Expected {
    Headers(Vec<(&'static str, &'static str)>),
    Session(Option<&'static str>),
    ValidName(&'static str),
    Error(AffinityError),
}

fn empty_params() -> ParamsSpec {
    ParamsSpec {
        header: None,
        session: None,
        litellm_session: None,
        metadata: None,
        litellm_metadata: None,
    }
}

fn affinity_params(spec: &ParamsSpec) -> AffinityParams {
    let metadata = spec.metadata.as_ref().map(|meta| SessionMetadata {
        session_id: meta.session.map(str::to_owned),
        trace_id: None,
        litellm_session_id_generated: meta.generated,
        other: IndexMap::default(),
    });
    let litellm_metadata = spec.litellm_metadata.as_ref().map(|meta| SessionMetadata {
        session_id: meta.session.map(str::to_owned),
        trace_id: None,
        litellm_session_id_generated: meta.generated,
        other: IndexMap::default(),
    });
    AffinityParams {
        provider_affinity_header: spec.header.map(str::to_owned),
        session_id: spec.session.map(str::to_owned),
        litellm_session_id: spec.litellm_session.map(str::to_owned),
        metadata,
        litellm_metadata,
    }
}

#[rstest]
#[case::adds_the_session_header(
    Scenario::Add {
        headers: &[],
        params: ParamsSpec { header: Some("x-session"), session: Some("s"), litellm_session: None, metadata: None, litellm_metadata: None },
    },
    Expected::Headers(vec![("x-session", "s")])
)]
#[case::adds_an_empty_configured_session_value(
    Scenario::Add {
        headers: &[],
        params: ParamsSpec { header: Some("x-session"), session: Some(""), litellm_session: Some("litellm"), metadata: None, litellm_metadata: None },
    },
    Expected::Headers(vec![("x-session", "")])
)]
#[case::existing_header_with_the_same_case_wins(
    Scenario::Add {
        headers: &[("x-session", "caller")],
        params: ParamsSpec { header: Some("x-session"), session: Some("s"), litellm_session: None, metadata: None, litellm_metadata: None },
    },
    Expected::Headers(vec![("x-session", "caller")])
)]
#[case::existing_header_with_another_case_wins(
    Scenario::Add {
        headers: &[("X-Session", "caller")],
        params: ParamsSpec { header: Some("x-session"), session: Some("s"), litellm_session: None, metadata: None, litellm_metadata: None },
    },
    Expected::Headers(vec![("X-Session", "caller")])
)]
#[case::unrelated_header_is_kept(
    Scenario::Add {
        headers: &[("other", "value")],
        params: ParamsSpec { header: Some("x-session"), session: Some("s"), litellm_session: None, metadata: None, litellm_metadata: None },
    },
    Expected::Headers(vec![("other", "value"), ("x-session", "s")])
)]
#[case::no_session_adds_nothing(
    Scenario::Add {
        headers: &[],
        params: ParamsSpec { header: Some("x-session"), session: None, litellm_session: None, metadata: None, litellm_metadata: None },
    },
    Expected::Headers(vec![])
)]
#[case::no_configured_header_is_a_no_op(
    Scenario::Add { headers: &[("other", "value")], params: empty_params() },
    Expected::Headers(vec![("other", "value")])
)]
#[case::carriage_return_is_rejected(
    Scenario::Add {
        headers: &[],
        params: ParamsSpec { header: Some("x-session"), session: Some("a\rb"), litellm_session: None, metadata: None, litellm_metadata: None },
    },
    Expected::Error(AffinityError::ControlCharacters)
)]
#[case::line_feed_is_rejected(
    Scenario::Add {
        headers: &[],
        params: ParamsSpec { header: Some("x-session"), session: Some("a\nb"), litellm_session: None, metadata: None, litellm_metadata: None },
    },
    Expected::Error(AffinityError::ControlCharacters)
)]
#[case::nul_is_rejected(
    Scenario::Add {
        headers: &[],
        params: ParamsSpec { header: Some("x-session"), session: Some("a\0b"), litellm_session: None, metadata: None, litellm_metadata: None },
    },
    Expected::Error(AffinityError::ControlCharacters)
)]
#[case::authentication_header_is_rejected(
    Scenario::Add {
        headers: &[],
        params: ParamsSpec { header: Some("Authorization"), session: Some("s"), litellm_session: None, metadata: None, litellm_metadata: None },
    },
    Expected::Error(AffinityError::ForbiddenHeader)
)]
#[case::cookie_header_is_rejected(
    Scenario::Add {
        headers: &[],
        params: ParamsSpec { header: Some("x-api-key"), session: Some("s"), litellm_session: None, metadata: None, litellm_metadata: None },
    },
    Expected::Error(AffinityError::ForbiddenHeader)
)]
#[case::invalid_header_name_is_rejected(
    Scenario::Add {
        headers: &[],
        params: ParamsSpec { header: Some("x session"), session: Some("s"), litellm_session: None, metadata: None, litellm_metadata: None },
    },
    Expected::Error(AffinityError::InvalidHeaderName)
)]
#[case::empty_header_name_is_rejected(
    Scenario::Add {
        headers: &[],
        params: ParamsSpec { header: Some(""), session: Some("s"), litellm_session: None, metadata: None, litellm_metadata: None },
    },
    Expected::Error(AffinityError::InvalidHeaderName)
)]
#[case::non_ascii_header_name_is_rejected(
    Scenario::Validate("\u{e9}"),
    Expected::Error(AffinityError::InvalidHeaderName)
)]
#[case::header_name_with_a_space_is_rejected(
    Scenario::Validate("x session"),
    Expected::Error(AffinityError::InvalidHeaderName)
)]
#[case::valid_header_name_is_kept(
    Scenario::Validate("X-Session"),
    Expected::ValidName("X-Session")
)]
#[case::direct_session_id_wins(
    Scenario::Session(ParamsSpec { header: None, session: Some("direct"), litellm_session: Some("litellm"), metadata: Some(MetaSpec { session: Some("metadata"), generated: Some(true) }), litellm_metadata: None }),
    Expected::Session(Some("direct"))
)]
#[case::litellm_session_id_wins_over_metadata(
    Scenario::Session(ParamsSpec { header: None, session: None, litellm_session: Some("litellm"), metadata: Some(MetaSpec { session: Some("metadata"), generated: Some(false) }), litellm_metadata: None }),
    Expected::Session(Some("litellm"))
)]
#[case::generated_litellm_session_id_is_skipped(
    Scenario::Session(ParamsSpec { header: None, session: None, litellm_session: Some("litellm"), metadata: Some(MetaSpec { session: None, generated: Some(true) }), litellm_metadata: Some(MetaSpec { session: Some("stable"), generated: None }) }),
    Expected::Session(Some("stable"))
)]
#[case::generated_metadata_is_skipped(
    Scenario::Session(ParamsSpec { header: None, session: None, litellm_session: None, metadata: Some(MetaSpec { session: Some("generated"), generated: Some(true) }), litellm_metadata: Some(MetaSpec { session: Some("stable"), generated: None }) }),
    Expected::Session(Some("stable"))
)]
#[case::metadata_session_id_is_used(
    Scenario::Session(ParamsSpec { header: None, session: None, litellm_session: None, metadata: Some(MetaSpec { session: Some("metadata"), generated: None }), litellm_metadata: Some(MetaSpec { session: Some("litellm-metadata"), generated: None }) }),
    Expected::Session(Some("metadata"))
)]
#[case::generated_litellm_metadata_skips_litellm_session_id(
    Scenario::Session(ParamsSpec { header: None, session: None, litellm_session: Some("litellm"), metadata: None, litellm_metadata: Some(MetaSpec { session: None, generated: Some(true) }) }),
    Expected::Session(None)
)]
#[case::generated_litellm_metadata_keeps_a_stable_metadata_session(
    Scenario::Session(ParamsSpec { header: None, session: None, litellm_session: Some("litellm"), metadata: Some(MetaSpec { session: Some("stable"), generated: None }), litellm_metadata: Some(MetaSpec { session: None, generated: Some(true) }) }),
    Expected::Session(Some("stable"))
)]
#[case::all_sessions_generated_gives_none(
    Scenario::Session(ParamsSpec { header: None, session: None, litellm_session: Some("litellm"), metadata: Some(MetaSpec { session: Some("generated"), generated: Some(true) }), litellm_metadata: Some(MetaSpec { session: None, generated: Some(true) }) }),
    Expected::Session(None)
)]
#[case::no_sessions_gives_none(Scenario::Session(empty_params()), Expected::Session(None))]
fn provider_controls_tests_behavior(#[case] scenario: Scenario, #[case] expected: Expected) {
    match scenario {
        Scenario::Add { headers, params } => {
            let configured: IndexMap<String, String> = headers
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect();
            let affinity = affinity_params(&params);
            let outcome = add_provider_affinity_header(&configured, &affinity);
            match expected {
                Expected::Headers(expected_headers) => {
                    let updated = outcome.expect("the affinity header is added");
                    let rendered: Vec<(&str, &str)> = updated
                        .iter()
                        .map(|(name, value)| (name.as_str(), value.as_str()))
                        .collect();
                    assert_eq!(rendered, expected_headers);
                }
                Expected::Error(expected_error) => {
                    assert_eq!(
                        outcome.expect_err("the affinity header is rejected"),
                        expected_error
                    );
                }
                Expected::Session(_) | Expected::ValidName(_) => {
                    panic!("this scenario has no matching expectation")
                }
            }
        }
        Scenario::Session(params) => {
            let affinity = affinity_params(&params);
            let Expected::Session(expected_session) = expected else {
                panic!("a session scenario expects a session")
            };
            assert_eq!(
                get_stable_session_id(&affinity).as_deref(),
                expected_session
            );
        }
        Scenario::Validate(name) => match expected {
            Expected::ValidName(expected_name) => {
                assert_eq!(
                    validate_affinity_header(name).expect("the name is valid"),
                    expected_name
                );
            }
            Expected::Error(expected_error) => {
                assert_eq!(
                    validate_affinity_header(name).expect_err("the name is rejected"),
                    expected_error
                );
            }
            Expected::Headers(_) | Expected::Session(_) => {
                panic!("a validation scenario has no matching expectation")
            }
        },
    }
}
