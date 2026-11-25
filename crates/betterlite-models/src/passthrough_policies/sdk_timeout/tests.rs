#![cfg(test)]

use rstest::rstest;
use serde_json::json;

use super::{
    DeploymentTimeouts, RequestTimeouts, RouterTimeouts, TimeoutError, TimeoutValue,
    resolve_sdk_timeout, timeout_value,
};

struct Case {
    request: RequestTimeouts,
    deployment: DeploymentTimeouts,
    router: RouterTimeouts,
    configured: Option<f64>,
    settings: f64,
}

impl Case {
    fn resolve(&self) -> Result<f64, TimeoutError> {
        resolve_sdk_timeout(
            &self.request,
            &self.deployment,
            &self.router,
            self.configured,
            self.settings,
        )
    }
}

fn number(value: f64) -> TimeoutValue {
    TimeoutValue::Number(value)
}

fn text(value: &str) -> TimeoutValue {
    TimeoutValue::Text(value.to_owned())
}

fn request(stream: bool) -> RequestTimeouts {
    RequestTimeouts {
        stream,
        stream_timeout: None,
        timeout: None,
        request_timeout: None,
    }
}

fn empty_deployment() -> DeploymentTimeouts {
    DeploymentTimeouts {
        stream_timeout: None,
        timeout: None,
        request_timeout: None,
    }
}

fn empty_router() -> RouterTimeouts {
    RouterTimeouts {
        stream_timeout: None,
        timeout: None,
    }
}

#[test]
fn an_absent_or_null_timeout_level_is_unset() {
    let absent = timeout_value(None);
    let null = timeout_value(Some(&json!(null)));
    assert_eq!(absent, None);
    assert_eq!(null, None);
}

#[test]
fn a_number_timeout_level_keeps_its_seconds() {
    let fraction = timeout_value(Some(&json!(12.5_f64)));
    let whole = timeout_value(Some(&json!(30_i32)));
    assert_eq!(fraction, Some(number(12.5_f64)));
    assert_eq!(whole, Some(number(30.0_f64)));
}

#[test]
fn a_bool_timeout_level_reads_as_one_or_zero_seconds() {
    let enabled = timeout_value(Some(&json!(true)));
    let disabled = timeout_value(Some(&json!(false)));
    assert_eq!(enabled, Some(number(1.0_f64)));
    assert_eq!(disabled, Some(number(0.0_f64)));
}

#[test]
fn a_text_timeout_level_keeps_its_text() {
    let written = timeout_value(Some(&json!("30")));
    assert_eq!(written, Some(text("30")));
}

#[test]
fn a_list_or_object_timeout_level_stays_its_json_text() {
    let list = timeout_value(Some(&json!([1_i32, 2_i32])));
    let object = timeout_value(Some(&serde_json::from_str(r#"{"a": 1}"#).unwrap()));
    assert_eq!(list, Some(text("[1,2]")));
    assert_eq!(object, Some(text(r#"{"a":1}"#)));
}

#[test]
fn request_stream_timeout_wins_over_every_lower_level() {
    let case = Case {
        request: RequestTimeouts {
            stream: true,
            stream_timeout: Some(number(5.0_f64)),
            timeout: Some(number(6.0_f64)),
            request_timeout: Some(number(7.0_f64)),
        },
        deployment: DeploymentTimeouts {
            stream_timeout: Some(number(8.0_f64)),
            timeout: Some(number(9.0_f64)),
            request_timeout: Some(number(10.0_f64)),
        },
        router: RouterTimeouts {
            stream_timeout: Some(number(11.0_f64)),
            timeout: Some(number(12.0_f64)),
        },
        configured: Some(13.0_f64),
        settings: 14.0_f64,
    };
    assert_eq!(case.resolve(), Ok(5.0_f64));
}

#[test]
fn deployment_stream_timeout_beats_the_router_and_the_generic_chain() {
    let case = Case {
        request: request(true),
        deployment: DeploymentTimeouts {
            stream_timeout: Some(number(8.0_f64)),
            timeout: Some(number(9.0_f64)),
            request_timeout: Some(number(10.0_f64)),
        },
        router: RouterTimeouts {
            stream_timeout: Some(number(11.0_f64)),
            timeout: Some(number(12.0_f64)),
        },
        configured: Some(13.0_f64),
        settings: 14.0_f64,
    };
    assert_eq!(case.resolve(), Ok(8.0_f64));
}

#[test]
fn router_stream_timeout_beats_the_generic_chain() {
    let case = Case {
        request: request(true),
        deployment: empty_deployment(),
        router: RouterTimeouts {
            stream_timeout: Some(number(11.0_f64)),
            timeout: Some(number(12.0_f64)),
        },
        configured: Some(13.0_f64),
        settings: 14.0_f64,
    };
    assert_eq!(case.resolve(), Ok(11.0_f64));
}

#[test]
fn stream_timeouts_are_ignored_without_a_streaming_request() {
    let case = Case {
        request: RequestTimeouts {
            stream: false,
            stream_timeout: Some(number(5.0_f64)),
            timeout: Some(number(6.0_f64)),
            request_timeout: None,
        },
        deployment: DeploymentTimeouts {
            stream_timeout: Some(number(8.0_f64)),
            timeout: None,
            request_timeout: None,
        },
        router: RouterTimeouts {
            stream_timeout: Some(number(11.0_f64)),
            timeout: None,
        },
        configured: None,
        settings: 600.0_f64,
    };
    assert_eq!(case.resolve(), Ok(6.0_f64));
}

#[test]
fn request_timeout_beats_request_request_timeout() {
    let case = Case {
        request: RequestTimeouts {
            stream: false,
            stream_timeout: None,
            timeout: Some(number(6.0_f64)),
            request_timeout: Some(number(7.0_f64)),
        },
        deployment: empty_deployment(),
        router: empty_router(),
        configured: None,
        settings: 600.0_f64,
    };
    assert_eq!(case.resolve(), Ok(6.0_f64));
}

#[test]
fn request_request_timeout_beats_the_deployment_values() {
    let case = Case {
        request: RequestTimeouts {
            stream: false,
            stream_timeout: None,
            timeout: None,
            request_timeout: Some(number(7.0_f64)),
        },
        deployment: DeploymentTimeouts {
            stream_timeout: Some(number(8.0_f64)),
            timeout: Some(number(9.0_f64)),
            request_timeout: Some(number(10.0_f64)),
        },
        router: RouterTimeouts {
            stream_timeout: None,
            timeout: Some(number(12.0_f64)),
        },
        configured: Some(13.0_f64),
        settings: 600.0_f64,
    };
    assert_eq!(case.resolve(), Ok(7.0_f64));
}

#[test]
fn deployment_timeout_beats_deployment_request_timeout() {
    let case = Case {
        request: request(false),
        deployment: DeploymentTimeouts {
            stream_timeout: None,
            timeout: Some(number(9.0_f64)),
            request_timeout: Some(number(10.0_f64)),
        },
        router: RouterTimeouts {
            stream_timeout: None,
            timeout: Some(number(12.0_f64)),
        },
        configured: None,
        settings: 600.0_f64,
    };
    assert_eq!(case.resolve(), Ok(9.0_f64));
}

#[test]
fn deployment_request_timeout_beats_the_router_timeout() {
    let case = Case {
        request: request(false),
        deployment: DeploymentTimeouts {
            stream_timeout: None,
            timeout: None,
            request_timeout: Some(number(10.0_f64)),
        },
        router: RouterTimeouts {
            stream_timeout: None,
            timeout: Some(number(12.0_f64)),
        },
        configured: Some(13.0_f64),
        settings: 600.0_f64,
    };
    assert_eq!(case.resolve(), Ok(10.0_f64));
}

#[test]
fn router_timeout_beats_the_configured_request_timeout() {
    let case = Case {
        request: request(false),
        deployment: empty_deployment(),
        router: RouterTimeouts {
            stream_timeout: None,
            timeout: Some(number(12.0_f64)),
        },
        configured: Some(13.0_f64),
        settings: 14.0_f64,
    };
    assert_eq!(case.resolve(), Ok(12.0_f64));
}

#[test]
fn the_configured_request_timeout_beats_the_general_setting() {
    let case = Case {
        request: request(false),
        deployment: empty_deployment(),
        router: empty_router(),
        configured: Some(13.0_f64),
        settings: 14.0_f64,
    };
    assert_eq!(case.resolve(), Ok(13.0_f64));
}

#[test]
fn an_explicit_default_request_timeout_is_still_used() {
    let case = Case {
        request: request(false),
        deployment: empty_deployment(),
        router: empty_router(),
        configured: Some(6_000.0_f64),
        settings: 14.0_f64,
    };
    assert_eq!(case.resolve(), Ok(6_000.0_f64));
}

#[test]
fn without_a_winner_the_settings_timeout_is_returned() {
    let case = Case {
        request: request(false),
        deployment: empty_deployment(),
        router: empty_router(),
        configured: None,
        settings: 14.0_f64,
    };
    assert_eq!(case.resolve(), Ok(14.0_f64));
}

#[test]
fn an_invalid_lower_precedence_value_never_fails_the_call() {
    let case = Case {
        request: RequestTimeouts {
            stream: false,
            stream_timeout: None,
            timeout: Some(number(5.0_f64)),
            request_timeout: Some(text("not-a-number")),
        },
        deployment: DeploymentTimeouts {
            stream_timeout: None,
            timeout: Some(text("also-not-a-number")),
            request_timeout: None,
        },
        router: RouterTimeouts {
            stream_timeout: None,
            timeout: Some(text("bad")),
        },
        configured: None,
        settings: 600.0_f64,
    };
    assert_eq!(case.resolve(), Ok(5.0_f64));
}

#[test]
fn an_invalid_lower_precedence_stream_value_never_fails_the_call() {
    let case = Case {
        request: RequestTimeouts {
            stream: true,
            stream_timeout: Some(number(5.0_f64)),
            timeout: None,
            request_timeout: None,
        },
        deployment: DeploymentTimeouts {
            stream_timeout: Some(text("bad")),
            timeout: None,
            request_timeout: None,
        },
        router: empty_router(),
        configured: None,
        settings: 600.0_f64,
    };
    assert_eq!(case.resolve(), Ok(5.0_f64));
}

#[test]
fn an_invalid_winner_names_its_request_field() {
    let case = Case {
        request: RequestTimeouts {
            stream: false,
            stream_timeout: None,
            timeout: Some(text("bad")),
            request_timeout: None,
        },
        deployment: empty_deployment(),
        router: empty_router(),
        configured: None,
        settings: 600.0_f64,
    };
    assert_eq!(
        case.resolve(),
        Err(TimeoutError::InvalidSeconds {
            field: "kwargs.timeout",
            value: "bad".to_owned(),
        })
    );
}

#[test]
fn an_invalid_stream_winner_names_its_field() {
    let case = Case {
        request: RequestTimeouts {
            stream: true,
            stream_timeout: Some(text("bad")),
            timeout: None,
            request_timeout: None,
        },
        deployment: empty_deployment(),
        router: empty_router(),
        configured: None,
        settings: 600.0_f64,
    };
    assert_eq!(
        case.resolve(),
        Err(TimeoutError::InvalidSeconds {
            field: "kwargs.stream_timeout",
            value: "bad".to_owned(),
        })
    );
}

#[test]
fn an_invalid_deployment_winner_names_its_field() {
    let case = Case {
        request: request(false),
        deployment: DeploymentTimeouts {
            stream_timeout: None,
            timeout: None,
            request_timeout: Some(text("abc")),
        },
        router: empty_router(),
        configured: None,
        settings: 600.0_f64,
    };
    assert_eq!(
        case.resolve(),
        Err(TimeoutError::InvalidSeconds {
            field: "litellm_params.request_timeout",
            value: "abc".to_owned(),
        })
    );
}

#[rstest]
#[case::integer_text("30", Ok(30.0_f64))]
#[case::fraction_text("12.5", Ok(12.5_f64))]
#[case::padded_text(" 12.5 ", Ok(12.5_f64))]
#[case::exponent_text("1e3", Ok(1_000.0_f64))]
#[case::signed_text("+3.0", Ok(3.0_f64))]
#[case::leading_dot(".5", Ok(0.5_f64))]
#[case::trailing_dot("5.", Ok(5.0_f64))]
#[case::single_underscore("1_0", Ok(10.0_f64))]
#[case::infinity("inf", Ok(f64::INFINITY))]
#[case::negative("-5", Ok(-5.0_f64))]
#[case::empty_text("", Err(()))]
#[case::letters("abc", Err(()))]
#[case::double_underscore("1__0", Err(()))]
#[case::trailing_underscore("1_", Err(()))]
#[case::padded_underscore(" 1_0 ", Err(()))]
#[case::hexadecimal("0x10", Err(()))]
fn the_winning_text_is_validated_as_seconds(
    #[case] written: &str,
    #[case] expected: Result<f64, ()>,
) {
    let case = Case {
        request: RequestTimeouts {
            stream: false,
            stream_timeout: None,
            timeout: Some(text(written)),
            request_timeout: None,
        },
        deployment: empty_deployment(),
        router: empty_router(),
        configured: None,
        settings: 600.0_f64,
    };
    match expected {
        Ok(seconds) => assert_eq!(case.resolve(), Ok(seconds)),
        Err(()) => assert!(matches!(
            case.resolve(),
            Err(TimeoutError::InvalidSeconds { .. })
        )),
    }
}
