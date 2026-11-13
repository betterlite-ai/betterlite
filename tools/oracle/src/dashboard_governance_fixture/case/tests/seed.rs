pub(super) const SEED_PREFIX: &str = "name: governance-fixture\nconfig: config\nenv: {}\nrequest: {method: GET, path: /health/liveliness, headers: {}, body: null}\nupstream: []\ndatabase:\n  seed:\n    users: [{user_id: alice}, {user_id: bob}]\n    keys: [{token: sk-alice-token, user_id: alice}, {token: sk-bob-token, user_id: bob}]\n    teams: [{team_id: team-red}]\n    memberships: [{user_id: alice, team_id: team-red}]\n    dashboard:\n      logs:\n        - {request_id: req-blocked, call_type: acompletion, start_time: '2026-01-04T10:00:00.000', end_time: '2026-01-04T10:00:00.500', key_index: 0, status: failure}\n        - {request_id: req-tools, call_type: acompletion, start_time: '2026-01-04T11:00:00.000', end_time: '2026-01-04T11:00:00.400', key_index: 1, status: success}\n";

pub(super) const CASE_SUFFIX: &str =
    "\ncompare: {headers: [], headers_present: [], upstream_headers: [], ignore: []}\n";

pub(super) fn case_with(governance: &str) -> String {
    format!("{SEED_PREFIX}{governance}{CASE_SUFFIX}")
}
