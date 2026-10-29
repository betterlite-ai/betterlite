#![cfg(test)]

use std::fs;
use std::path::Path;

use super::decoder::{DbError, decode_database_url};
use super::flag::{FlagValue, TokenFlagError, str_to_bool, token_auth_flag_enabled};
use super::policy::{
    ConfiguredSeconds, DatabaseTimeoutSettings, PoolTimeouts, TimeoutError,
    build_db_connection_url_params, pg_options_with_timeouts, unsupported_db_scheme,
    unsupported_db_scheme_message,
};
use super::query::{
    QueryValue, add_missing_query_params, append_query_params, connection_params_from_url,
    idle_lifetime_params, reader_shareable_params, token_refresh_params_from_url, url_query_value,
    with_query_value,
};

#[test]
fn missing_only_identity() {
    let url = "postgresql://h/db?a=1&b=2";
    let params = vec![("a".to_owned(), QueryValue::Integer(9))];
    assert_eq!(add_missing_query_params(url, &params), url);
}

#[test]
fn blank_pinned_value() {
    let url = "postgresql://h/db?a=&c=1";
    let params = vec![
        ("a".to_owned(), QueryValue::Text("9".to_owned())),
        ("b".to_owned(), QueryValue::Text("2".to_owned())),
    ];
    assert_eq!(
        add_missing_query_params(url, &params),
        "postgresql://h/db?a=&c=1&b=2"
    );
}

#[test]
fn duplicate_query_order() {
    let url = "postgresql://h/db?x=1&x=2&y=3";
    let params = vec![("z".to_owned(), QueryValue::Integer(4))];
    assert_eq!(
        add_missing_query_params(url, &params),
        "postgresql://h/db?x=1&x=2&y=3&z=4"
    );
}

#[test]
fn idle_none_zero_negative() {
    assert_eq!(
        idle_lifetime_params(None),
        vec![(
            "max_idle_connection_lifetime".to_owned(),
            QueryValue::Integer(60)
        )]
    );
    assert_eq!(
        idle_lifetime_params(Some(0.0_f64)),
        vec![(
            "max_idle_connection_lifetime".to_owned(),
            QueryValue::Float(0.0)
        )]
    );
    assert_eq!(
        idle_lifetime_params(Some(-5.0_f64)),
        vec![(
            "max_idle_connection_lifetime".to_owned(),
            QueryValue::Float(-5.0)
        )]
    );
}

#[test]
fn reader_exact_allowlist() {
    let params = vec![
        ("connection_limit".to_owned(), QueryValue::Integer(1)),
        ("pool_timeout".to_owned(), QueryValue::Integer(2)),
        ("connect_timeout".to_owned(), QueryValue::Integer(3)),
        ("socket_timeout".to_owned(), QueryValue::Integer(4)),
        (
            "max_idle_connection_lifetime".to_owned(),
            QueryValue::Integer(5),
        ),
        ("pgbouncer".to_owned(), QueryValue::Text("true".to_owned())),
        ("schema".to_owned(), QueryValue::Text("public".to_owned())),
        ("sslmode".to_owned(), QueryValue::Text("require".to_owned())),
        ("options".to_owned(), QueryValue::Text("-c".to_owned())),
    ];
    assert_eq!(reader_shareable_params(&params), params[..6].to_vec());
}

#[test]
fn reader_last_duplicate_no_blanks() {
    let url = "postgresql://h/db?connection_limit=1&connection_limit=2&pool_timeout=&schema=public";
    assert_eq!(
        connection_params_from_url(url),
        vec![(
            "connection_limit".to_owned(),
            QueryValue::Text("2".to_owned())
        )]
    );
}

#[test]
fn refresh_tls_allowlist() {
    let url = "postgresql://h/db?connection_limit=5&sslmode=verify-full&sslcert=/path&sslaccept=strict&sslrootcert=/root&schema=public&options=-c";
    assert_eq!(
        token_refresh_params_from_url(url),
        vec![
            (
                "connection_limit".to_owned(),
                QueryValue::Text("5".to_owned())
            ),
            (
                "sslmode".to_owned(),
                QueryValue::Text("verify-full".to_owned())
            ),
            ("sslcert".to_owned(), QueryValue::Text("/path".to_owned())),
            (
                "sslaccept".to_owned(),
                QueryValue::Text("strict".to_owned())
            ),
        ]
    );
}

#[test]
fn pool_optional_omission() {
    let params = build_db_connection_url_params(10, PoolTimeouts::default(), false, &[]);
    assert_eq!(
        params,
        vec![("connection_limit".to_owned(), QueryValue::Integer(10))]
    );
}

#[test]
fn extra_params_override() {
    let extra = vec![
        ("connection_limit".to_owned(), QueryValue::Integer(99)),
        ("new_key".to_owned(), QueryValue::Text("v".to_owned())),
    ];
    let params = build_db_connection_url_params(10, PoolTimeouts::default(), false, &extra);
    assert_eq!(
        params,
        vec![
            ("connection_limit".to_owned(), QueryValue::Integer(99)),
            ("new_key".to_owned(), QueryValue::Text("v".to_owned())),
        ]
    );
}

#[test]
fn append_overwrites_lists() {
    let url = "postgresql://h/db?tag=old1&tag=old2";
    let params = vec![("tag".to_owned(), vec!["new1".to_owned(), "new2".to_owned()])];
    let outcome = append_query_params(Some(url), &params);
    assert_eq!(outcome.url, "postgresql://h/db?tag=new1&tag=new2");
    assert!(outcome.warning.is_none());
}

#[test]
fn append_empty_warns() {
    let none_outcome = append_query_params(None, &[]);
    assert_eq!(none_outcome.url, "");
    assert_eq!(
        none_outcome.warning,
        Some("append_query_params received empty or non-string URL, returning empty string")
    );
    let empty_outcome = append_query_params(Some(""), &[]);
    assert_eq!(empty_outcome.url, "");
    assert_eq!(
        empty_outcome.warning,
        Some("append_query_params received empty or non-string URL, returning empty string")
    );
}

#[test]
fn first_query_value() {
    let url = "postgresql://h/db?k=&k=first&k=second";
    assert_eq!(url_query_value(Some(url), "k"), "first");
    assert_eq!(url_query_value(Some(url), "missing"), "");
    assert_eq!(url_query_value(None, "k"), "");
}

#[test]
fn replace_all_query_values() {
    let url = "postgresql://h/db?k=old1&k=old2&j=3";
    assert_eq!(
        with_query_value(url, "k", "new"),
        "postgresql://h/db?j=3&k=new"
    );
}

#[test]
fn options_seconds_to_millis() {
    let result = pg_options_with_timeouts("", Some(1.5_f64), None).unwrap();
    assert_eq!(result, "-c statement_timeout=1500");
}

#[test]
fn options_existing_spellings() {
    for existing in [
        "-c statement_timeout=100",
        "-cstatement_timeout=100",
        "--statement_timeout=100",
    ] {
        let result = pg_options_with_timeouts(existing, Some(5.0_f64), None).unwrap();
        assert_eq!(result, existing);
    }
}

#[test]
fn options_zero_negative() {
    let result = pg_options_with_timeouts("", Some(0.0_f64), Some(-1.0_f64)).unwrap();
    assert_eq!(result, "-c statement_timeout=0 -c lock_timeout=-1000");
}

#[test]
fn invalid_timeout_no_fallback() {
    let result =
        DatabaseTimeoutSettings::validate(Some(ConfiguredSeconds::Text("abc".to_owned())), None);
    assert!(matches!(result, Err(TimeoutError::NotNumeric { .. })));
}

#[test]
fn nonfinite_timeout_fails() {
    let nan_result = pg_options_with_timeouts("", Some(f64::NAN), None);
    assert!(matches!(nan_result, Err(TimeoutError::NotANumber)));
    let infinite_result = pg_options_with_timeouts("", Some(f64::INFINITY), None);
    assert!(matches!(infinite_result, Err(TimeoutError::Infinite)));
}

#[test]
fn postgres_scheme_aliases() {
    assert_eq!(unsupported_db_scheme("postgres://h/db"), None);
    assert_eq!(unsupported_db_scheme("postgresql://h/db"), None);
    assert_eq!(unsupported_db_scheme("POSTGRESQL://h/db"), None);
    assert_eq!(unsupported_db_scheme("POSTGRES://h/db"), None);
}

#[test]
fn unsupported_scheme_message() {
    let message = unsupported_db_scheme_message("DATABASE_URL", "mysql");
    assert_eq!(
        message,
        "DATABASE_URL uses unsupported scheme 'mysql'. LiteLLM's database features (virtual keys, store_model_in_db, spend tracking) require PostgreSQL; use a 'postgresql://' connection string. SQLite and other engines are not supported. See https://docs.litellm.ai/docs/proxy/virtual_keys"
    );
}

#[test]
fn malformed_dsn_scheme() {
    assert_eq!(
        unsupported_db_scheme("user:pass@host/db"),
        Some("user".to_owned())
    );
}

#[test]
fn token_boolean_spellings() {
    for truthy in ["1", "on", "t", "true", "y", "yes", "TRUE", " True "] {
        let value = FlagValue::Text(truthy.to_owned());
        assert!(
            token_auth_flag_enabled(Some(&value), "ENV").unwrap(),
            "{truthy}"
        );
    }
    for falsy in ["", "0", "f", "false", "n", "no", "off", "FALSE"] {
        let value = FlagValue::Text(falsy.to_owned());
        assert!(
            !token_auth_flag_enabled(Some(&value), "ENV").unwrap(),
            "{falsy}"
        );
    }
    assert!(token_auth_flag_enabled(Some(&FlagValue::Bool(true)), "ENV").unwrap());
    assert!(!token_auth_flag_enabled(None, "ENV").unwrap());
}

#[test]
fn token_invalid_flag_message() {
    let value = FlagValue::Text("maybe".to_owned());
    let error = token_auth_flag_enabled(Some(&value), "IAM_TOKEN_DB_AUTH").unwrap_err();
    let TokenFlagError::NotRecognized { message } = error;
    assert_eq!(
        message,
        "IAM_TOKEN_DB_AUTH='maybe' is not a recognized boolean. Set it to one of 1, on, t, true, y, yes to turn it on, or to one of 0, f, false, n, no, off to turn it off."
    );
}

#[test]
fn extras_boolean_spellings() {
    for truthy in ["true", "1", "t", "y", "yes", "TRUE"] {
        assert!(str_to_bool(Some(truthy)), "{truthy}");
    }
    assert!(!str_to_bool(Some("false")));
    assert!(!str_to_bool(None));
}

#[test]
fn extras_whitespace_and_on_false() {
    assert!(!str_to_bool(Some(" true ")));
    assert!(!str_to_bool(Some("on")));
    assert!(!str_to_bool(Some("ON")));
}

#[test]
fn malformed_url_bytes() {
    assert!(matches!(
        decode_database_url(b"postgresql://[::1/db"),
        Err(DbError::MalformedBracket)
    ));
    assert!(matches!(
        decode_database_url(b"postgresql://host:abc/db"),
        Err(DbError::MalformedPort { .. })
    ));
    assert!(matches!(
        decode_database_url(b"postgresql://user%2name@host/db"),
        Err(DbError::MalformedPercentEscape)
    ));
    assert!(matches!(
        decode_database_url(b"localhost/db"),
        Err(DbError::MissingScheme)
    ));
    assert!(matches!(
        decode_database_url(&[0xff, 0xfe]),
        Err(DbError::InvalidUtf8 { .. })
    ));
}

#[test]
fn native_url_size_limit() {
    let oversized = vec![b'a'; 0x0010_0001];
    assert!(matches!(
        decode_database_url(&oversized),
        Err(DbError::TooLarge { len: 0x0010_0001 })
    ));
    let boundary = vec![b'a'; 0x0010_0000];
    assert!(matches!(
        decode_database_url(&boundary),
        Err(DbError::MissingScheme)
    ));
}

#[test]
fn native_url_debug_redaction() {
    let endpoint = decode_database_url(b"postgresql://alice:s3cr3t@host/db").unwrap();
    let debug = format!("{endpoint:?}");
    assert!(!debug.contains("alice"));
    assert!(!debug.contains("s3cr3t"));
    assert!(debug.contains("REDACTED"));
}

#[test]
fn replays_the_database_urls_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/database_urls");
    for entry in fs::read_dir(corpus).unwrap() {
        let path = entry.unwrap().path();
        let bytes = fs::read(&path).unwrap();
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        let decoded = decode_database_url(&bytes);
        assert_eq!(decoded.is_err(), name.ends_with("-rejected"), "{name}");
    }
}
