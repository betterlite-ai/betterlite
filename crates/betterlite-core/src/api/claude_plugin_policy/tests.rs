#![cfg(test)]

use std::collections::BTreeSet;

use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use rstest::rstest;

use super::source::{
    ARCHIVE_SHA256_INVALID, ARCHIVE_URL_INVALID, GIT_SUBDIR_PATH_INVALID, GIT_SUBDIR_PATH_MISSING,
    GIT_SUBDIR_URL_MISSING, GITHUB_REPO_MISSING, PluginSource, PluginSpec, UNKNOWN_SOURCE,
    URL_FIELD_MISSING, UpdatePluginSpec, build_plugin_manifest, is_https_url_with_host,
    validate_plugin_source,
};
use super::{
    ClaudePluginPolicyError, ClaudePluginPolicyInput, DB_NOT_CONNECTED, NOT_PROXY_ADMIN,
    PluginCaller, PluginObjectPermission, PluginRecord, PluginWhere, evaluate, granted_skills,
    name_conflict_error, plugin_not_found_error, require_database, require_proxy_admin,
    skill_visibility,
};

const BASE_PRINCIPAL: &str = r#"{"principal_type":"human","subject":"s","auth_method":"api_key"}"#;
const ADMIN: &str = "proxy_admin";

fn identity(role: Option<&str>) -> Principal {
    let mut identity: Principal = serde_json::from_str(BASE_PRINCIPAL).unwrap();
    identity.roles = match role {
        Some(ADMIN) => vec![Role::PlatformAdmin],
        Some("proxy_admin_viewer") => vec![Role::PlatformViewer],
        _ => Vec::new(),
    };
    identity
}

fn permission(values: &[&str]) -> PluginObjectPermission {
    PluginObjectPermission {
        skills: Some(values.iter().map(|value| (*value).to_owned()).collect()),
    }
}

fn caller(role: Option<&str>, key: Option<&[&str]>, team: Option<&[&str]>) -> PluginCaller {
    PluginCaller {
        principal: identity(role),
        object_permission: key.map(permission),
        team_object_permission: team.map(permission),
    }
}

fn source(json: &str) -> PluginSource {
    serde_json::from_str(json).unwrap()
}

#[rstest]
#[case::https_with_host("https://bucket.s3.amazonaws.com/plugin.zip", true)]
#[case::uppercase_scheme("HTTPS://bucket.s3.amazonaws.com/plugin.zip", true)]
#[case::with_userinfo("https://user:secret@bucket/plugin.zip", true)]
#[case::plain_http("http://bucket/plugin.zip", false)]
#[case::hostless("https:///plugin.zip", false)]
#[case::empty("", false)]
#[case::no_scheme("bucket/plugin.zip", false)]
#[case::other_scheme("ftp://bucket/plugin.zip", false)]
#[case::no_slashes("https:bucket/plugin.zip", false)]
#[case::one_slash("https:/bucket/plugin.zip", false)]
#[case::backslashes(r"https:\\bucket\plugin.zip", false)]
#[case::scheme_only("https://", false)]
#[case::leading_space(" https://bucket/plugin.zip", true)]
#[case::leading_control("\u{1}https://bucket/plugin.zip", true)]
#[case::tab_between_slashes("https:/\t/bucket/plugin.zip", true)]
#[case::trailing_newline("https://bucket/plugin.zip\n", true)]
fn https_url_with_host(#[case] url: &str, #[case] expected: bool) {
    assert_eq!(is_https_url_with_host(url), expected);
}

#[rstest]
#[case::https_archive(
    r#"{"source":"archive","url":"https://bucket.s3.amazonaws.com/plugins/plugin-name.zip"}"#,
    None
)]
#[case::https_archive_with_sha(
    r#"{"source":"archive","url":"https://host/plugin.zip","sha256":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"}"#,
    None
)]
#[case::uppercase_sha(
    r#"{"source":"archive","url":"https://host/plugin.zip","sha256":"0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF"}"#,
    None
)]
#[case::http_archive(
    r#"{"source":"archive","url":"http://host/plugin.zip"}"#,
    Some(ARCHIVE_URL_INVALID)
)]
#[case::hostless_archive(
    r#"{"source":"archive","url":"https:///plugin.zip"}"#,
    Some(ARCHIVE_URL_INVALID)
)]
#[case::missing_archive_url(r#"{"source":"archive"}"#, Some(ARCHIVE_URL_INVALID))]
#[case::archive_without_slashes(
    r#"{"source":"archive","url":"https:host/p.zip"}"#,
    Some(ARCHIVE_URL_INVALID)
)]
#[case::archive_with_one_slash(
    r#"{"source":"archive","url":"https:/host/p.zip"}"#,
    Some(ARCHIVE_URL_INVALID)
)]
#[case::archive_with_leading_space(r#"{"source":"archive","url":" https://host/p.zip"}"#, None)]
#[case::short_sha(
    r#"{"source":"archive","url":"https://host/plugin.zip","sha256":"abc"}"#,
    Some(ARCHIVE_SHA256_INVALID)
)]
#[case::empty_sha(
    r#"{"source":"archive","url":"https://host/plugin.zip","sha256":""}"#,
    Some(ARCHIVE_SHA256_INVALID)
)]
fn plugin_https_source(#[case] source_json: &str, #[case] expected: Option<&str>) {
    let rejection = validate_plugin_source(&source(source_json))
        .err()
        .map(|error| error.fields());
    let message = rejection
        .as_ref()
        .map(|fields| fields.detail.error.as_str());
    assert_eq!(message, expected);
    assert_eq!(
        rejection.map(|fields| fields.status_code),
        expected.map(|_| 400)
    );
}

#[rstest]
#[case::github_needs_repo(r#"{"source":"github"}"#, Some(GITHUB_REPO_MISSING))]
#[case::github_empty_repo_is_present(r#"{"source":"github","repo":""}"#, None)]
#[case::github_with_extra_key(r#"{"source":"github","repo":"org/repo","ref":"main"}"#, None)]
#[case::url_needs_url(r#"{"source":"url"}"#, Some(URL_FIELD_MISSING))]
#[case::url_accepts_any_string(r#"{"source":"url","url":"not a url"}"#, None)]
#[case::subdir_needs_url(r#"{"source":"git-subdir"}"#, Some(GIT_SUBDIR_URL_MISSING))]
#[case::subdir_empty_url(r#"{"source":"git-subdir","url":""}"#, Some(GIT_SUBDIR_URL_MISSING))]
#[case::subdir_needs_path(
    r#"{"source":"git-subdir","url":"https://host/repo.git"}"#,
    Some(GIT_SUBDIR_PATH_MISSING)
)]
#[case::subdir_empty_path(
    r#"{"source":"git-subdir","url":"https://host/repo.git","path":""}"#,
    Some(GIT_SUBDIR_PATH_MISSING)
)]
#[case::subdir_nested_path(
    r#"{"source":"git-subdir","url":"https://host/repo.git","path":"plugins/my-plugin"}"#,
    None
)]
#[case::subdir_parent_escape(
    r#"{"source":"git-subdir","url":"https://host/repo.git","path":"../x"}"#,
    Some(GIT_SUBDIR_PATH_INVALID)
)]
#[case::subdir_inner_escape(
    r#"{"source":"git-subdir","url":"https://host/repo.git","path":"plugins/../x"}"#,
    Some(GIT_SUBDIR_PATH_INVALID)
)]
#[case::subdir_leading_slash(
    r#"{"source":"git-subdir","url":"https://host/repo.git","path":"/plugins"}"#,
    Some(GIT_SUBDIR_PATH_INVALID)
)]
#[case::subdir_double_slash(
    r#"{"source":"git-subdir","url":"https://host/repo.git","path":"a//b"}"#,
    Some(GIT_SUBDIR_PATH_INVALID)
)]
#[case::subdir_trailing_slash(
    r#"{"source":"git-subdir","url":"https://host/repo.git","path":"plugins/"}"#,
    Some(GIT_SUBDIR_PATH_INVALID)
)]
#[case::subdir_leading_dot(
    r#"{"source":"git-subdir","url":"https://host/repo.git","path":".hidden"}"#,
    Some(GIT_SUBDIR_PATH_INVALID)
)]
#[case::subdir_space(
    r#"{"source":"git-subdir","url":"https://host/repo.git","path":"a b"}"#,
    Some(GIT_SUBDIR_PATH_INVALID)
)]
#[case::subdir_underscore_segment(
    r#"{"source":"git-subdir","url":"https://host/repo.git","path":"a_b/c-d.e"}"#,
    None
)]
#[case::subdir_trailing_newline(
    r#"{"source":"git-subdir","url":"https://host/repo.git","path":"plugins/x\n"}"#,
    Some(GIT_SUBDIR_PATH_INVALID)
)]
#[case::sha_trailing_newline(
    r#"{"source":"archive","url":"https://host/p.zip","sha256":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\n"}"#,
    Some(ARCHIVE_SHA256_INVALID)
)]
#[case::unknown_source(r#"{"source":"svn"}"#, Some(UNKNOWN_SOURCE))]
#[case::missing_source(r#"{"repo":"org/repo"}"#, Some(UNKNOWN_SOURCE))]
#[case::capitalised_source(r#"{"source":"GitHub","repo":"org/repo"}"#, Some(UNKNOWN_SOURCE))]
fn invalid_plugin_source(#[case] source_json: &str, #[case] expected: Option<&str>) {
    match validate_plugin_source(&source(source_json)) {
        Ok(()) => assert!(expected.is_none(), "{source_json} was accepted"),
        Err(error) => assert_eq!(
            Some(error.fields().detail.error),
            expected.map(str::to_owned)
        ),
    }
}

#[rstest]
#[case::intersection(Some(&["a", "b"]), Some(&["b", "c"]), &["b"])]
#[case::key_only(Some(&["a"]), None, &["a"])]
#[case::team_only(None, Some(&["b"]), &["b"])]
#[case::neither(None, None, &[])]
#[case::empty_key_defers_to_team(Some(&[]), Some(&["b"]), &["b"])]
#[case::empty_team_defers_to_key(Some(&["a"]), Some(&[]), &["a"])]
#[case::both_empty(Some(&[]), Some(&[]), &[])]
#[case::disjoint(Some(&["a"]), Some(&["b"]), &[])]
#[case::duplicates_collapse(Some(&["a", "a"]), None, &["a"])]
fn skill_object_permission(
    #[case] key: Option<&[&str]>,
    #[case] team: Option<&[&str]>,
    #[case] expected: &[&str],
) {
    let want: BTreeSet<String> = expected.iter().map(|name| (*name).to_owned()).collect();
    assert_eq!(granted_skills(&caller(None, key, team)), want);
}

#[rstest]
#[case::enabled_plugin_is_public(false, true, true)]
#[case::disabled_plugin_is_hidden(false, false, false)]
#[case::granted_disabled_plugin(false, false, true)]
#[case::admin_sees_private_plugins(true, false, false)]
fn skill_visibility_allows(#[case] admin: bool, #[case] enabled: bool, #[case] granted: bool) {
    let role = admin.then_some(ADMIN);
    let key: Option<&[&str]> = granted.then_some(&["private"]);
    let visibility = skill_visibility(Some(&caller(role, key, None)));
    let plugin = PluginRecord {
        name: "private".to_owned(),
        enabled,
    };
    let expected = enabled || admin || granted;
    assert_eq!(visibility.allows(&plugin), expected);
}

#[test]
fn unauthenticated_callers_see_only_public_plugins() {
    let visibility = skill_visibility(None);
    assert_eq!(visibility, super::PUBLIC_ONLY);
    assert!(visibility.allows(&PluginRecord {
        name: "public".to_owned(),
        enabled: true,
    }));
    assert!(!visibility.allows(&PluginRecord {
        name: "private".to_owned(),
        enabled: false,
    }));
}

#[rstest]
#[case::public_only(None, None, r#"{"enabled":true}"#)]
#[case::granted(
    None,
    Some(&["b", "a"]),
    r#"{"OR":[{"enabled":true},{"name":{"in":["a","b"]}}]}"#
)]
#[case::admin_queries_everything(Some(ADMIN), None, r#"{}"#)]
fn skill_visibility_where(
    #[case] role: Option<&str>,
    #[case] key: Option<&[&str]>,
    #[case] expected: &str,
) {
    let input = ClaudePluginPolicyInput {
        caller: Some(caller(role, key, None)),
        plugins: Vec::new(),
    };
    let result = evaluate(&input);
    assert!(result.visible.is_empty());
    assert_eq!(result.visibility.sees_private, role == Some(ADMIN));
    assert_eq!(serde_json::to_string(&result.filter).unwrap(), expected);
}

#[rstest]
#[case::admin(Some(ADMIN), None)]
#[case::viewer(Some("proxy_admin_viewer"), Some(403))]
#[case::internal(Some("internal_user"), Some(403))]
#[case::anonymous(None, Some(403))]
fn nonadmin_plugin_denied(#[case] role: Option<&str>, #[case] expected: Option<u16>) {
    let caller = role.map(|role| self::caller(Some(role), None, None));
    let result = require_proxy_admin(caller.as_ref());
    assert_eq!(result.is_ok(), expected.is_none());
    let fields = result.err().map(|error| error.fields());
    assert_eq!(fields.as_ref().map(|fields| fields.status_code), expected);
    let detail = fields.map(|fields| fields.detail.error);
    assert_eq!(detail, expected.map(|_| NOT_PROXY_ADMIN.to_owned()));
}

#[test]
fn a_nonadmin_caller_is_refused_before_the_database_is_consulted() {
    let anonymous = None;
    let admin_denied = require_proxy_admin(anonymous);
    assert_eq!(
        admin_denied.err().map(|error| error.fields().status_code),
        Some(403)
    );
    let admin = caller(Some(ADMIN), None, None);
    assert!(require_proxy_admin(Some(&admin)).is_ok());
    let database_down = require_database(false).err().map(|error| error.fields());
    assert_eq!(
        database_down.as_ref().map(|fields| fields.status_code),
        Some(500)
    );
    assert_eq!(
        database_down.map(|fields| fields.detail.error),
        Some(DB_NOT_CONNECTED.to_owned())
    );
    assert!(require_database(true).is_ok());
}

#[test]
fn the_error_envelope_names_the_error_key() {
    let conflict = name_conflict_error("my-plugin").fields();
    assert_eq!(conflict.status_code, 409);
    assert_eq!(
        serde_json::to_string(&conflict.detail).unwrap(),
        r#"{"error":"A skill named 'my-plugin' already exists. Update the existing skill instead of adding it again."}"#
    );
    let missing = plugin_not_found_error("my-plugin").fields();
    assert_eq!(missing.status_code, 404);
    assert_eq!(
        serde_json::to_string(&missing.detail).unwrap(),
        r#"{"error":"Plugin 'my-plugin' not found"}"#
    );
}

#[rstest]
#[case::default_version(
    r#"{"source":{"source":"github","repo":"org/repo"}}"#,
    r#"{"name":"my-plugin","source":{"source":"github","repo":"org/repo"},"version":"1.0.0"}"#
)]
#[case::empty_fields_dropped(
    r#"{"source":{"source":"github","repo":"org/repo"},"version":"","description":"","homepage":"","keywords":[],"category":"","domain":"","namespace":""}"#,
    r#"{"name":"my-plugin","source":{"source":"github","repo":"org/repo"}}"#
)]
#[case::every_option_kept(
    r#"{"source":{"source":"github","repo":"org/repo"},"version":"2.0.0","description":"desc","homepage":"https://h","keywords":["k"],"category":"cat","domain":"dom","namespace":"ns"}"#,
    r#"{"name":"my-plugin","source":{"source":"github","repo":"org/repo"},"version":"2.0.0","description":"desc","homepage":"https://h","keywords":["k"],"category":"cat","domain":"dom","namespace":"ns"}"#
)]
#[case::author_email_omitted(
    r#"{"source":{"source":"github","repo":"org/repo"},"version":"2.0.0","author":{"name":"Ada"},"keywords":["git"]}"#,
    r#"{"name":"my-plugin","source":{"source":"github","repo":"org/repo"},"version":"2.0.0","author":{"name":"Ada"},"keywords":["git"]}"#
)]
#[case::author_url_dropped(
    r#"{"source":{"source":"github","repo":"org/repo"},"author":{"name":"Ada","url":"https://ada.dev"}}"#,
    r#"{"name":"my-plugin","source":{"source":"github","repo":"org/repo"},"version":"1.0.0","author":{"name":"Ada"}}"#
)]
#[case::git_subdir_source_keeps_url_and_path(
    r#"{"source":{"source":"git-subdir","url":"https://host/repo.git","path":"plugins/x"}}"#,
    r#"{"name":"my-plugin","source":{"source":"git-subdir","url":"https://host/repo.git","path":"plugins/x"},"version":"1.0.0"}"#
)]
#[case::archive_source_keeps_url_and_sha256(
    r#"{"source":{"source":"archive","url":"https://host/p.zip","sha256":"0000000000000000000000000000000000000000000000000000000000000000"}}"#,
    r#"{"name":"my-plugin","source":{"source":"archive","url":"https://host/p.zip","sha256":"0000000000000000000000000000000000000000000000000000000000000000"},"version":"1.0.0"}"#
)]
#[case::author_email_kept(
    r#"{"source":{"source":"github","repo":"org/repo"},"author":{"name":"Ada","email":"ada@example.com"}}"#,
    r#"{"name":"my-plugin","source":{"source":"github","repo":"org/repo"},"version":"1.0.0","author":{"name":"Ada","email":"ada@example.com"}}"#
)]
#[case::extra_source_keys_survive(
    r#"{"source":{"source":"github","repo":"org/repo","ref":"main"}}"#,
    r#"{"name":"my-plugin","source":{"source":"github","repo":"org/repo","ref":"main"},"version":"1.0.0"}"#
)]
fn manifest_holds_the_present_spec_fields(#[case] spec: &str, #[case] expected: &str) {
    let spec: PluginSpec = serde_json::from_str(spec).unwrap();
    let manifest = build_plugin_manifest("my-plugin", &spec);
    assert_eq!(serde_json::to_string(&manifest).unwrap(), expected);
}

#[test]
fn an_update_clears_an_omitted_version() {
    let update: UpdatePluginSpec = serde_json::from_str(
        r#"{"name":"renamed","source":{"source":"url","url":"https://host/repo.git"}}"#,
    )
    .unwrap();
    let manifest = build_plugin_manifest("my-plugin", &PluginSpec::from(update));
    assert_eq!(
        serde_json::to_string(&manifest).unwrap(),
        r#"{"name":"my-plugin","source":{"source":"url","url":"https://host/repo.git"}}"#
    );
}

#[test]
fn an_update_keeps_every_field_it_carries() {
    let update: UpdatePluginSpec = serde_json::from_str(
        r#"{"source":{"source":"github","repo":"org/repo"},"version":"2.0.0","description":"desc","author":{"name":"Ada","email":"ada@example.com"},"homepage":"https://h","keywords":["k"],"category":"cat","domain":"dom","namespace":"ns"}"#,
    )
    .unwrap();
    let manifest = build_plugin_manifest("my-plugin", &PluginSpec::from(update));
    assert_eq!(
        serde_json::to_string(&manifest).unwrap(),
        r#"{"name":"my-plugin","source":{"source":"github","repo":"org/repo"},"version":"2.0.0","description":"desc","author":{"name":"Ada","email":"ada@example.com"},"homepage":"https://h","keywords":["k"],"category":"cat","domain":"dom","namespace":"ns"}"#
    );
}

#[test]
fn evaluate_keeps_only_the_plugins_the_caller_may_see() {
    let input = ClaudePluginPolicyInput {
        caller: Some(caller(None, Some(&["private"]), None)),
        plugins: vec![
            PluginRecord {
                name: "public".to_owned(),
                enabled: true,
            },
            PluginRecord {
                name: "private".to_owned(),
                enabled: false,
            },
            PluginRecord {
                name: "other".to_owned(),
                enabled: false,
            },
        ],
    };
    let result = evaluate(&input);
    assert_eq!(
        result.visible,
        vec!["public".to_owned(), "private".to_owned()]
    );
    assert_eq!(
        result.filter,
        PluginWhere {
            enabled: None,
            any: Some(vec![
                super::PluginWhereBranch::Enabled { enabled: true },
                super::PluginWhereBranch::Name {
                    name: super::NameFilter {
                        names: vec!["private".to_owned()],
                    },
                },
            ]),
        }
    );
    assert_eq!(
        result.visibility.granted,
        ["private".to_owned()].into_iter().collect()
    );
}

#[test]
fn an_admin_sees_every_plugin() {
    let input = ClaudePluginPolicyInput {
        caller: Some(caller(Some(ADMIN), None, None)),
        plugins: vec![PluginRecord {
            name: "private".to_owned(),
            enabled: false,
        }],
    };
    let result = evaluate(&input);
    assert_eq!(result.visible, vec!["private".to_owned()]);
    assert_eq!(
        result.filter,
        PluginWhere {
            enabled: None,
            any: None
        }
    );
}
