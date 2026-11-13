#![cfg(test)]

use rstest::rstest;

use super::{ConsentPage, SUCCESS_PAGE, render_consent_page, render_verification_page};

const VERIFY_URL: &str = "https://proxy.example.com/sso/cli/start";
const TOKEN: &str = "browser-token";
const ORIGIN: &str = "http://127.0.0.1:53682";
const USER: &str = "user@example.com";
const FLOW: &str = "flow-handle";
const COMPLETE: &str = "https://proxy.example.com/authorize/complete";

fn consent_page(teams: &[(String, String)]) -> String {
    render_consent_page(ConsentPage {
        client_origin: ORIGIN,
        user_id: USER,
        teams,
        flow_handle: FLOW,
        complete_url: COMPLETE,
        cli_expiration_hours: 24,
    })
}

fn assert_unbranded(html: &str) {
    assert!(!html.contains("LiteLLM"));
    assert!(!html.contains("LITELLM"));
}

#[test]
fn verification_page_posts_its_token_and_code_to_the_supplied_action() {
    let html = render_verification_page(VERIFY_URL, TOKEN, None);
    assert!(html.contains(&format!(r#"<form method="post" action="{VERIFY_URL}">"#)));
    assert!(html.contains(&format!(
        r#"<input type="hidden" name="browser_complete_token" value="{TOKEN}" />"#
    )));
    assert!(html.contains(r#"<label for="user_code">Verification code</label>"#));
    assert!(html.contains(
        r#"<input id="user_code" name="user_code" autocomplete="one-time-code" required autofocus />"#
    ));
    assert!(html.contains(r#"<button type="submit">Continue</button>"#));
}

#[test]
fn a_prefill_fills_the_value_attribute_and_switches_the_instructions() {
    let html = render_verification_page(VERIFY_URL, TOKEN, Some("K7QP-3M2N"));
    assert!(html.contains(
        r#"<input id="user_code" name="user_code" autocomplete="one-time-code" value="K7QP-3M2N" required autofocus />"#
    ));
    assert!(html.contains("<p>Confirm the verification code below to finish this login.</p>"));
    assert!(!html.contains("Enter the verification code shown in your terminal"));
}

#[test]
fn an_absent_or_empty_prefill_leaves_the_input_empty() {
    let absent = render_verification_page(VERIFY_URL, TOKEN, None);
    let empty = render_verification_page(VERIFY_URL, TOKEN, Some(""));
    let input = r#"<input id="user_code" name="user_code" autocomplete="one-time-code" required autofocus />"#;
    let instructions =
        "<p>Enter the verification code shown in your terminal to finish this login.</p>";
    assert!(absent.contains(input));
    assert!(empty.contains(input));
    assert!(!empty.contains("value=\"\""));
    assert!(absent.contains(instructions));
    assert!(empty.contains(instructions));
    assert!(!empty.contains("Confirm the verification code below"));
}

#[test]
fn verification_page_escapes_hostile_action_token_and_prefill() {
    let html = render_verification_page(
        "https://proxy.example.com/a?x=\"y\"&z=<w>'v",
        "t\"o'k&<>",
        Some("A\"B'C&D<E>F"),
    );
    assert!(html.contains(
        r#"action="https://proxy.example.com/a?x=&quot;y&quot;&amp;z=&lt;w&gt;&#x27;v""#
    ));
    assert!(html.contains(r#"value="t&quot;o&#x27;k&amp;&lt;&gt;""#));
    assert!(html.contains(r#"value="A&quot;B&#x27;C&amp;D&lt;E&gt;F""#));
    assert!(!html.contains("<w>"));
}

#[test]
fn success_page_keeps_the_close_script_and_the_terminal_instructions() {
    let html = SUCCESS_PAGE;
    assert!(html.contains("<script>\n            window.close();\n        </script>"));
    assert!(html.contains(
        "Return to your terminal - the CLI will automatically detect the successful authentication."
    ));
    assert!(html.contains("You can now close this window and return to your terminal."));
    assert!(html.contains("<h1>Authentication Successful!</h1>"));
}

#[test]
fn pages_show_the_betterlite_name_and_never_the_litellm_one() {
    let verification = render_verification_page(VERIFY_URL, TOKEN, None);
    let success = SUCCESS_PAGE;
    let consent = consent_page(&[]);
    assert!(verification.contains("<title>betterlite CLI Login</title>"));
    assert!(success.contains("<title>CLI Authentication Successful - betterlite</title>"));
    assert!(success.contains("&#128645; betterlite"));
    // the banner's first row opens with the B and E glyphs of BETTERLITE
    assert!(success.contains(
        "<div class=\"banner\">   \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2557} \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2557}"
    ));
    assert!(success.contains(
        "<p>Your betterlite CLI has been successfully authenticated and is ready to use.</p>"
    ));
    assert!(success.contains(
        "<p>You can now use betterlite CLI commands with your authenticated session.</p>"
    ));
    assert!(consent.contains("<title>Authorize CLI access - betterlite</title>"));
    assert!(consent.contains("wants to call betterlite as"));
    assert_unbranded(&verification);
    assert_unbranded(success);
    assert_unbranded(&consent);
}

#[test]
fn consent_page_renders_no_team_field_for_zero_teams() {
    let html = consent_page(&[]);
    assert!(!html.contains("team_id"));
    assert!(!html.contains("Attribute requests to team"));
    assert!(!html.contains("Requests are attributed to team"));
}

#[test]
fn consent_page_hides_the_only_team_and_names_it() {
    let teams = [("team-1".to_owned(), "Team One".to_owned())];
    let html = consent_page(&teams);
    assert!(html.contains(
        r#"<input type="hidden" name="team_id" value="team-1"><p>Requests are attributed to team <strong>Team One</strong>.</p>"#
    ));
    assert!(!html.contains("<select"));
}

#[test]
fn consent_page_orders_a_select_for_several_teams() {
    let teams = [
        ("team-1".to_owned(), "Team One".to_owned()),
        ("team-2".to_owned(), "Team Two".to_owned()),
    ];
    let html = consent_page(&teams);
    assert!(html.contains(
        r#"<label for="team_id">Attribute requests to team</label><select id="team_id" name="team_id"><option value="team-1">Team One</option><option value="team-2">Team Two</option></select>"#
    ));
    assert!(!html.contains(r#"<input type="hidden" name="team_id""#));
}

#[test]
fn consent_page_escapes_team_ids_and_labels() {
    let teams = [("t\"1&<>".to_owned(), "L\"&<>'".to_owned())];
    let html = consent_page(&teams);
    assert!(html.contains(r#"value="t&quot;1&amp;&lt;&gt;""#));
    assert!(html.contains("<strong>L&quot;&amp;&lt;&gt;&#x27;</strong>"));
}

#[test]
fn consent_page_shows_the_origin_user_lifetime_and_decision_buttons() {
    let html = render_consent_page(ConsentPage {
        client_origin: ORIGIN,
        user_id: USER,
        teams: &[],
        flow_handle: FLOW,
        complete_url: COMPLETE,
        cli_expiration_hours: 36,
    });
    assert!(html.contains("<code>http://127.0.0.1:53682</code>"));
    assert!(html.contains("<strong>user@example.com</strong>"));
    assert!(html.contains("expires within 36 hours"));
    assert!(
        html.contains(
            r#"<form method="post" action="https://proxy.example.com/authorize/complete">"#
        )
    );
    assert!(html.contains(r#"<input type="hidden" name="flow" value="flow-handle">"#));
    assert!(html.contains(
        r#"<button type="submit" name="decision" value="deny" class="deny">Deny</button>"#
    ));
    assert!(html.contains(
        r#"<button type="submit" name="decision" value="approve" class="approve">Approve</button>"#
    ));
}

#[test]
fn consent_page_escapes_quotes_in_the_origin_user_action_and_flow() {
    let html = render_consent_page(ConsentPage {
        client_origin: "http://127.0.0.1/\"x\"&<>",
        user_id: "u\"'&<>",
        teams: &[],
        flow_handle: "f\"&<>'",
        complete_url: "https://proxy.example.com/complete?a=\"b\"&c=<d>'e",
        cli_expiration_hours: 24,
    });
    assert!(html.contains("<code>http://127.0.0.1/&quot;x&quot;&amp;&lt;&gt;</code>"));
    assert!(html.contains("<strong>u&quot;&#x27;&amp;&lt;&gt;</strong>"));
    assert!(html.contains(r#"name="flow" value="f&quot;&amp;&lt;&gt;&#x27;""#));
    assert!(html.contains(
        r#"action="https://proxy.example.com/complete?a=&quot;b&quot;&amp;c=&lt;d&gt;&#x27;e""#
    ));
}

#[test]
fn the_flow_handle_travels_only_in_the_form_body() {
    let html = consent_page(&[]);
    assert_eq!(html.matches(FLOW).count(), 1);
    assert!(html.contains(r#"value="flow-handle""#));
    assert!(html.contains(&format!(r#"<form method="post" action="{COMPLETE}">"#)));
}

#[test]
fn only_the_consent_page_declares_the_no_referrer_policy() {
    assert!(consent_page(&[]).contains(r#"<meta name="referrer" content="no-referrer">"#));
    assert!(!render_verification_page(VERIFY_URL, TOKEN, None).contains("referrer"));
    assert!(!SUCCESS_PAGE.contains("referrer"));
}

#[rstest]
#[case::ampersand("&", "&amp;")]
#[case::less_than("<", "&lt;")]
#[case::greater_than(">", "&gt;")]
#[case::double_quote("\"", "&quot;")]
#[case::apostrophe("'", "&#x27;")]
#[case::unchanged("plain 123", "plain 123")]
#[case::mixed("a&b<c>d\"e'f", "a&amp;b&lt;c&gt;d&quot;e&#x27;f")]
fn escaping_matches_python_html_escape(#[case] raw: &str, #[case] escaped: &str) {
    assert_eq!(super::escape::text(raw), escaped);
}

#[test]
fn a_value_that_spells_a_marker_is_not_replaced_twice() {
    let html = render_verification_page("https://proxy.example.com/@@BROWSER_TOKEN@@", TOKEN, None);
    assert!(html.contains("action=\"https://proxy.example.com/@@BROWSER_TOKEN@@\""));
    assert!(html.contains(&format!(r#"value="{TOKEN}""#)));
}
