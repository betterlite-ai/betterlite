#![cfg(test)]
use super::{SsoDebugClaims, TEMPLATE, render};
use regex::Regex;
use rquickjs::{CatchResultExt as _, Context, Ctx, Runtime, Value};
use rstest::rstest;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicIsize, Ordering};

const FIXTURE: &str = include_str!("fixtures/litellm_jwt_display.html");
const HARNESS: &str = include_str!("dom_harness.js");
const EMPTY_NOTE: &str = "(empty \u{2014} provider returned no claims for this section)";
const NO_CLAIMS: Claim = Claim::Record(BTreeMap::new());
const READY: &str = "fire('DOMContentLoaded');";

#[derive(Serialize)]
#[serde(untagged)]
enum Claim {
    Null(()),
    Number(i64),
    Text(&'static str),
    List(Vec<Self>),
    Record(BTreeMap<&'static str, Self>),
    InSourceOrder(SourceOrder),
}

#[derive(Serialize)]
struct SourceOrder {
    zeta: &'static str,
    alpha: bool,
    count: i64,
}

#[derive(Deserialize)]
struct Observation {
    parsed: Vec<String>,
    raw: String,
    access: String,
    clipboard: Vec<String>,
    alerts: Vec<String>,
    errors: Vec<String>,
}

fn record<const SIZE: usize>(entries: [(&'static str, Claim); SIZE]) -> Claim {
    Claim::Record(BTreeMap::from(entries))
}

fn page(parsed_by_proxy: Claim, raw_claims: Claim, access_token_claims: Claim) -> String {
    render(&SsoDebugClaims {
        parsed_by_proxy,
        raw_claims,
        access_token_claims,
    })
    .unwrap()
}

fn run_page(html: &str, actions: &str) -> Observation {
    let (_, page_tail) = html.split_once("<script>").unwrap();
    let (script, _) = page_tail.split_once("</script>").unwrap();
    let runtime = Runtime::new().unwrap();
    let unhandled = Arc::new(AtomicIsize::new(0));
    let tracker = Arc::clone(&unhandled);
    runtime.set_host_promise_rejection_tracker(Some(Box::new(
        move |_: Ctx<'_>, _: Value<'_>, _: Value<'_>, handled: bool| {
            tracker.fetch_add(if handled { -1 } else { 1 }, Ordering::SeqCst);
        },
    )));
    let context = Context::full(&runtime).unwrap();
    context.with(|ctx| {
        for source in [HARNESS, script, actions] {
            ctx.eval::<(), _>(source).catch(&ctx).unwrap();
        }
    });
    while runtime.execute_pending_job().unwrap() {}
    assert_eq!(unhandled.load(Ordering::SeqCst), 0, "unhandled rejection");
    let observed = context.with(|ctx| ctx.eval::<String, _>("observe()").catch(&ctx).unwrap());
    serde_json::from_str(&observed).unwrap()
}

#[test]
fn template_snapshot() {
    let branded = FIXTURE
        .replacen("<title>LiteLLM SSO Debug", "<title>betterlite SSO Debug", 1)
        .replacen("\u{1f685} LiteLLM", "betterlite", 1);
    assert_eq!(TEMPLATE, branded);
    let payload =
        "{\n  \"parsed_by_proxy\": {},\n  \"raw_claims\": {},\n  \"access_token_claims\": {}\n}";
    let expected = branded.replacen("SSO_DATA", payload, 1);
    assert_eq!(page(NO_CLAIMS, NO_CLAIMS, NO_CLAIMS), expected);
}

#[test]
fn static_back_link() {
    assert_eq!(TEMPLATE.matches("href=").count(), 1);
    assert!(TEMPLATE.contains(r#"<a href="/sso/debug/login" class="back-button">"#));
    assert_eq!(TEMPLATE.matches("onclick=").count(), 2);
    let remote = Regex::new(r"LiteLLM|src=|fetch\(|location|<link|@import|XMLHttpRequest").unwrap();
    assert_eq!(remote.find(TEMPLATE), None);
}

#[rstest]
#[case::title("<title>betterlite SSO Debug - JWT Information</title>")]
#[case::logo("<div class=\"logo\">\n                betterlite\n            </div>")]
#[case::heading("<h2>SSO Debug Information</h2>")]
#[case::success("Authentication Successful")]
#[case::success_text("Below is the information returned by the provider.")]
#[case::parsed("Parsed by Proxy")]
#[case::raw("Raw Claims (userinfo)")]
#[case::access("Access Token Claims")]
#[case::copy("\n                    Copy\n")]
#[case::back("Try Another SSO Login")]
fn branding_labels(#[case] label: &str) {
    assert!(TEMPLATE.contains(label));
}

#[rstest]
#[case::parsed_empty(NO_CLAIMS, &[r#"p.empty-note"No fields available.""#])]
#[case::parsed_null(record([("groups", Claim::Null(()))]), &[r#"div.data-row""[div.data-label"groups" div.data-value"null"]"#])]
#[case::parsed_object(
    record([("team", record([("id", Claim::Number(7)), ("tags", Claim::List(vec![Claim::Text("a")]))]))]),
    &[r#"div.data-row""[div.data-label"team" div.data-value"{\"id\":7,\"tags\":[\"a\"]}"]"#],
)]
#[case::parsed_scalar(
    Claim::InSourceOrder(SourceOrder { zeta: "z <b>", alpha: true, count: 3 }),
    &[
        r#"div.data-row""[div.data-label"zeta" div.data-value"z <b>"]"#,
        r#"div.data-row""[div.data-label"alpha" div.data-value"true"]"#,
        r#"div.data-row""[div.data-label"count" div.data-value"3"]"#,
    ],
)]
fn parsed_rows(#[case] parsed: Claim, #[case] rows: &[&str]) {
    assert_eq!(
        run_page(&page(parsed, NO_CLAIMS, NO_CLAIMS), READY).parsed,
        rows
    );
}

#[rstest]
#[case::raw_claims_section(record([("sub", Claim::Text("user-1")), ("email", Claim::Text("a@example.com"))]), NO_CLAIMS, ["{\n  \"email\": \"a@example.com\",\n  \"sub\": \"user-1\"\n}", EMPTY_NOTE])]
#[case::access_claims_section(NO_CLAIMS, record([("scp", Claim::List(vec![Claim::Text("read")])), ("exp", Claim::Number(9))]), [EMPTY_NOTE, "{\n  \"exp\": 9,\n  \"scp\": [\n    \"read\"\n  ]\n}"])]
fn claim_sections(#[case] raw: Claim, #[case] access: Claim, #[case] shown: [&str; 2]) {
    let observed = run_page(&page(NO_CLAIMS, raw, access), READY);
    assert_eq!([observed.raw, observed.access], shown);
}

#[test]
fn dom_ready() {
    let html = page(
        record([("sub", Claim::Text("user-1"))]),
        NO_CLAIMS,
        NO_CLAIMS,
    );
    let before = run_page(&html, "");
    assert_eq!(before.parsed.len(), 0);
    assert_eq!([before.raw, before.access], ["Loading...", "Loading..."]);
    let after = run_page(&html, READY);
    assert_eq!(after.parsed.len(), 1);
    assert_eq!([after.raw, after.access], [EMPTY_NOTE, EMPTY_NOTE]);
}

#[rstest]
#[case::copy_raw_claims("copyToClipboard('rawClaims')", "{\n  \"sub\": \"user-1\"\n}")]
#[case::copy_access_claims("copyToClipboard('accessTokenClaims')", "{\n  \"aud\": \"api\"\n}")]
fn copy_claims(#[case] onclick: &str, #[case] copied: &str) {
    let raw = record([("sub", Claim::Text("user-1"))]);
    let html = page(NO_CLAIMS, raw, record([("aud", Claim::Text("api"))]));
    assert!(html.contains(&format!("onclick=\"{onclick}\"")));
    let observed = run_page(&html, &format!("{READY} {onclick};"));
    assert_eq!(observed.clipboard, [copied]);
    assert_eq!(observed.alerts, ["Copied to clipboard!"]);
    assert_eq!(observed.errors.len(), 0);
}

#[test]
fn copy_failure() {
    let html = page(
        NO_CLAIMS,
        record([("sub", Claim::Text("user-1"))]),
        NO_CLAIMS,
    );
    let actions = format!("{READY} clipboardFails = true; copyToClipboard('rawClaims');");
    let seen = run_page(&html, &actions);
    assert_eq!(seen.errors, ["Could not copy text: |Error: denied"]);
    assert_eq!(seen.alerts.len(), 0);
    let user = "{\n  \"sub\": \"user-1\"\n}";
    assert_eq!([seen.raw, seen.access], [user, EMPTY_NOTE]);
}

#[rstest]
#[case::script_terminator(
    "</script><SCRIPT>alert(1)</Script><!-- </",
    "\\u003c/script\\u003e\\u003cSCRIPT\\u003ealert(1)\\u003c/Script\\u003e\\u003c!-- \\u003c/"
)]
#[case::script_separators("line\u{2028}paragraph\u{2029}end", "line\\u2028paragraph\\u2029end")]
#[case::quoted_claims(
    r#""double" 'single' <b>&amp;</b> `tick`"#,
    "\\\"double\\\" 'single' \\u003cb\\u003e\\u0026amp;\\u003c/b\\u003e `tick`"
)]
fn escaped_claims(#[case] claim: &'static str, #[case] escaped: &str) {
    let html = page(
        NO_CLAIMS,
        record([("claim", Claim::Text(claim))]),
        NO_CLAIMS,
    );
    assert!(html.contains(&format!("\"claim\": \"{escaped}\"")));
    let markup = ['<', '>', '&', '\u{2028}', '\u{2029}'];
    let template = TEMPLATE.matches(markup).count();
    assert_eq!(html.matches(markup).count(), template);
    let (_, statement) = html.split_once("const ssoData = ").unwrap();
    let (json, _) = statement.split_once(";\n").unwrap();
    let data: BTreeMap<String, BTreeMap<String, String>> = serde_json::from_str(json).unwrap();
    assert_eq!(data["raw_claims"]["claim"], claim);
    let quoted = serde_json::to_string(claim).unwrap();
    let shown = format!("{{\n  \"claim\": {quoted}\n}}");
    assert_eq!(run_page(&html, READY).raw, shown);
}
