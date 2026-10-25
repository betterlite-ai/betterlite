#![cfg(test)]

use rstest::rstest;

use crate::settings::SettingsError;
use crate::settings::github_url::normalize_github_api_url;

#[rstest]
#[case::default_host("https://api.github.com", Ok("https://api.github.com"))]
#[case::enterprise_path(
    " https://github.enterprise.com/api/// ",
    Ok("https://github.enterprise.com/api")
)]
#[case::upper_scheme("HTTPS://api.github.com", Ok("HTTPS://api.github.com"))]
#[case::empty_query_and_fragment("https://h.com/?#", Ok("https://h.com/?#"))]
#[case::empty_userinfo("https://@h.com", Ok("https://@h.com"))]
#[case::user_with_empty_password("https://:@h.com", Ok("https://:@h.com"))]
#[case::ipv6_host("https://[::1]:8443", Ok("https://[::1]:8443"))]
#[case::scoped_ipv6_host("https://[fe80::1%eth0]", Ok("https://[fe80::1%eth0]"))]
#[case::ipvfuture_host("https://[v1.x]", Ok("https://[v1.x]"))]
#[case::leading_control_characters("\u{1}https://h.com", Ok("\u{1}https://h.com"))]
#[case::empty("  / ", Err(SettingsError::MissingGitHubUrl))]
#[case::http("http://api.github.com", Err(SettingsError::GitHubUrl))]
#[case::no_host("https:///api", Err(SettingsError::GitHubUrl))]
#[case::no_slashes("https:api.github.com", Err(SettingsError::GitHubUrl))]
#[case::only_port("https://:443", Err(SettingsError::GitHubUrl))]
#[case::username("https://me@h.com", Err(SettingsError::GitHubUrl))]
#[case::password("https://:pw@h.com", Err(SettingsError::GitHubUrl))]
#[case::query("https://h.com?a=1", Err(SettingsError::GitHubUrl))]
#[case::fragment("https://h.com#top", Err(SettingsError::GitHubUrl))]
#[case::unclosed_bracket("https://[::1", Err(SettingsError::Ipv6Url))]
#[case::text_before_bracket("https://a[::1]", Err(SettingsError::Ipv6Url))]
#[case::text_after_bracket("https://[::1]x", Err(SettingsError::Ipv6Url))]
#[case::bad_ipvfuture("https://[vz.x]", Err(SettingsError::IpvFuture))]
#[case::bracketed_ipv4("https://[1.2.3.4]", Err(SettingsError::BracketedIpv4))]
#[case::bracketed_name("https://[abc]", Err(SettingsError::BracketedHost { host: "abc".to_owned() }))]
#[case::bracket_in_userinfo("https://[u]@h.com", Err(SettingsError::BracketedHost { host: "h.com".to_owned() }))]
#[case::nfkc_slash(
    "https://h\u{ff0f}x.com",
    Err(SettingsError::NetlocNormalization { netloc: "h\u{ff0f}x.com".to_owned() })
)]
fn github_url_validation(#[case] value: &str, #[case] expected: Result<&str, SettingsError>) {
    assert_eq!(normalize_github_api_url(value), expected.map(str::to_owned));
}
