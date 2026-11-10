use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use crate::web_identity::claims::decode_web_identity_claims;

fn token(payload: &str) -> String {
    let encoded = URL_SAFE_NO_PAD.encode(payload.as_bytes());
    format!("header.{encoded}.signature")
}

fn audience(claims: &str) -> Option<String> {
    decode_web_identity_claims(&token(claims))
}

pub(super) fn jwt_audience_hint() {
    assert_reported_claims();
    assert_repr_escapes();
}

fn assert_reported_claims() {
    assert_eq!(
        audience(r#"{"aud":"litellm"}"#).as_deref(),
        Some("aud='litellm', iss=None")
    );
    assert_eq!(
        audience(r#"{"aud":["a","b"]}"#).as_deref(),
        Some("aud=['a', 'b'], iss=None")
    );
    assert_eq!(
        audience(r#"{"iss":"https://issuer.example"}"#).as_deref(),
        Some("aud=None, iss='https://issuer.example'")
    );
    assert_eq!(
        audience(r#"{"aud":"","iss":""}"#).as_deref(),
        Some("aud='', iss=''")
    );
    assert_eq!(
        audience(r#"{"sub":"x","aud":"a"}"#).as_deref(),
        Some("aud='a', iss=None")
    );
    assert_eq!(
        audience(r#"{"aud":[]}"#).as_deref(),
        Some("aud=[], iss=None")
    );
}

fn assert_repr_escapes() {
    assert_eq!(
        audience(r#"{"aud":"it's"}"#).as_deref(),
        Some(r#"aud="it's", iss=None"#)
    );
    assert_eq!(
        audience(r#"{"aud":"it's \"x\""}"#).as_deref(),
        Some(r#"aud='it\'s "x"', iss=None"#)
    );
    assert_eq!(
        audience(r#"{"aud":"back\\slash\nline\ttab\rend"}"#).as_deref(),
        Some(r"aud='back\\slash\nline\ttab\rend', iss=None")
    );
    assert_eq!(
        audience("{\"aud\":\"h\\u00e9llo\"}").as_deref(),
        Some("aud='h\u{e9}llo', iss=None")
    );
    assert_eq!(
        audience("{\"aud\":\"nbsp\\u00a0end\"}").as_deref(),
        Some(r"aud='nbsp\xa0end', iss=None")
    );
    assert_eq!(
        audience("{\"aud\":\"sep\\u2028end\"}").as_deref(),
        Some(r"aud='sep\u2028end', iss=None")
    );
    assert_eq!(
        audience("{\"aud\":\"emoji \\ud83d\\ude00\"}").as_deref(),
        Some("aud='emoji \u{1F600}', iss=None")
    );
    assert_eq!(
        audience(r#"{"aud":["quote's","d\"q"]}"#).as_deref(),
        Some(r#"aud=["quote's", 'd"q'], iss=None"#)
    );
    assert_eq!(
        audience(r#"{"aud":"del\u007f"}"#).as_deref(),
        Some(r"aud='del\x7f', iss=None")
    );
}

pub(super) fn malformed_token() {
    assert_no_claims();
    assert_broken_tokens();
}

fn assert_no_claims() {
    for claims in [
        "{}",
        r#"{"aud":null,"iss":null}"#,
        r#"{"aud":5}"#,
        r#"{"aud":["a",5]}"#,
        r#"{"aud":true}"#,
        r#"{"iss":5}"#,
        r#"{"aud":{}}"#,
        "[]",
        "null",
        "{",
        r#"{"aud":"x"} trailing"#,
    ] {
        assert_eq!(audience(claims), None, "{claims}");
    }
}

fn assert_broken_tokens() {
    for raw in [
        "",
        "a.b",
        "a.b.c.d",
        "a..c",
        "a.!!!.c",
        "a.e30=.c",
        "a.Zm9v.c",
        "a.eyJhdWQiOiJ4I.c",
    ] {
        assert_eq!(decode_web_identity_claims(raw), None, "{raw}");
    }
    for (claims, tail, decodes) in [
        (r#"{"aud":"x"}"#, "!", false),
        (r#"{"aud":""}"#, "!!", false),
        (r#"{"aud":"x"}"#, "=", true),
        (r#"{"aud":""}"#, "==", true),
        (r#"{"aud":"xy"}"#, "!", true),
    ] {
        let raw = format!("h.{}{tail}.c", URL_SAFE_NO_PAD.encode(claims));
        assert_eq!(decode_web_identity_claims(&raw).is_some(), decodes, "{raw}");
    }
    assert_eq!(
        decode_web_identity_claims(".eyJhdWQiOiJ4In0.").as_deref(),
        Some("aud='x', iss=None")
    );
    let broken_utf8 = format!(
        "h.{}.c",
        URL_SAFE_NO_PAD.encode([b'{', b'"', b'a', b'u', b'd', b'"', b':', b'"', 0xff])
    );
    assert_eq!(decode_web_identity_claims(&broken_utf8), None);
}
