#![cfg(test)]

use rstest::rstest;
use support::{Show, Step, capture_logs, entry, observe, parsed, team_entry};

use super::ptu_warning::warn_unattributed_ptu;

mod support;

const NO_INFO: &str = r#"{"model_name":"g","litellm_params":{"model":"openai/m"}}"#;
const NUMBERED: &str =
    r#"{"model_name":"g","litellm_params":{"model":"openai/m"},"model_info":{"id":7}}"#;
const ORGANIZED: &str =
    r#"{"model_name":"g","litellm_params":{"model":"openai/m","organization":["o1","o2"]}}"#;

const TOP_LEVEL_LIMITS: &str = r#"{"model_name":"g","rpm":10,"tpm":20,"litellm_params":{"model":"openai/m"},"model_info":{"id":"a"}}"#;
const OWN_RPM: &str = r#"{"model_name":"g","rpm":10,"tpm":20,"litellm_params":{"model":"openai/m","rpm":5},"model_info":{"id":"a"}}"#;
const NULL_LIMITS: &str = r#"{"model_name":"g","rpm":null,"tpm":null,"litellm_params":{"model":"openai/m"},"model_info":{"id":"a"}}"#;
const SOURCES: &str = r#"{"model_name":"g","litellm_params":{"model":"openai/m","dataSources":[{"type":"t","parameters":{"endpoint":"os.environ/AZURE_ENDPOINT","key":"os.environ/AZURE_KEY","index_name":"os.environ/AZURE_KEY"}}]},"model_info":{"id":"a"}}"#;
const UNSET_SOURCES: &str = r#"{"model_name":"g","litellm_params":{"model":"openai/m","dataSources":[{"parameters":{"endpoint":"os.environ/MISSING","key":"os.environ/EMPTY","index_name":"plain"}}]},"model_info":{"id":"a"}}"#;
const ODD_SOURCES: &str = r#"{"model_name":"g","litellm_params":{"model":"openai/m","dataSources":[{"type":"t"},5,{"parameters":{"endpoint":"literal","key":7}},{"parameters":"text"}]},"model_info":{"id":"a"}}"#;

fn ptu(name: &str, info: &str) -> String {
    format!(
        r#"{{"model_name":"{name}","litellm_params":{{"model":"openai/m"}},"model_info":{info}}}"#
    )
}

fn warning(names: &str) -> String {
    if names.is_empty() {
        return String::new();
    }
    format!(
        "PTU fields are set on config.yaml deployment(s) {names}, but PTU cost attribution is disabled, so no flat cost accrues and this traffic is billed per token. Set LITELLM_ENABLE_PTU_COST_ATTRIBUTION=True to enable it"
    )
}

fn oversized() -> String {
    // a mebibyte of literal text is past the regex size limit, so the wildcard cannot compile
    entry(&format!("{}*", "a".repeat(1 << 20)), "w", "openai/m")
}

fn first() -> String {
    entry("g", "a", "openai/m1")
}

fn second() -> String {
    entry("g", "b", "openai/m2")
}

#[rstest]
#[case::duplicate_names_kept(vec![Step::Set(vec![first(), second()])], false, Show::Models, "models=[g:a,g:b] ids(a:0,b:1) names(g:[0, 1]) set=[\"g\"]")]
#[case::duplicate_ids_last_wins(vec![Step::Set(vec![first(), entry("h", "a", "openai/m")])], false, Show::Models, "models=[g:a,h:a] ids(a:1) names(g:[0],h:[1]) set=[\"g\",\"h\"]")]
#[case::set_replaces_the_list(vec![Step::Set(vec![first()]), Step::Set(vec![entry("h", "c", "openai/m")])], false, Show::Models, "models=[h:c] ids(c:0) names(h:[0]) set=[\"h\"]")]
#[case::empty_name_is_not_indexed(vec![Step::Set(vec![entry("", "a", "openai/m")])], false, Show::Models, "models=[:a] ids(a:0) names() set=[\"\"]")]
#[case::add_appends(vec![Step::Set(vec![first()]), Step::Add(second())], false, Show::Models, "models=[g:a,g:b] ids(a:0,b:1) names(g:[0, 1]) set=[\"g\"]")]
#[case::add_reports_the_deployment(vec![Step::Set(vec![first()]), Step::Add(second())], false, Show::Outcomes, "set:ok;add:Some(b)")]
#[case::add_known_id_is_none(vec![Step::Set(vec![first()]), Step::Add(entry("h", "a", "openai/m"))], false, Show::Outcomes, "set:ok;add:None")]
#[case::add_known_id_changes_nothing(vec![Step::Set(vec![first()]), Step::Add(entry("h", "a", "openai/m"))], false, Show::Models, "models=[g:a] ids(a:0) names(g:[0]) set=[\"g\"]")]
#[case::add_new_name_joins_the_set(vec![Step::Set(vec![first()]), Step::Add(entry("h", "c", "openai/m"))], false, Show::Models, "models=[g:a,h:c] ids(a:0,c:1) names(g:[0],h:[1]) set=[\"g\",\"h\"]")]
#[case::add_runs_the_effects_in_order(vec![Step::Add(first())], false, Show::Calls, "admit:g,registered:a,invalidated")]
#[case::upsert_adds_a_new_id(vec![Step::Upsert(first())], false, Show::Outcomes, "upsert:Some(a)")]
#[case::upsert_identical_is_none(vec![Step::Set(vec![first()]), Step::Upsert(first())], false, Show::Outcomes, "set:ok;upsert:None")]
#[case::upsert_identical_runs_no_effects(vec![Step::Set(vec![first()]), Step::Upsert(first())], false, Show::Calls, "admit:g,adaptive:1,invalidated")]
#[case::upsert_changed_params_replaces_at_the_end(vec![Step::Set(vec![first(), second()]), Step::Upsert(entry("g", "a", "openai/m3"))], false, Show::Models, "models=[g:b,g:a] ids(a:1,b:0) names(g:[0, 1]) set=[\"g\"]")]
#[case::upsert_changed_name_is_a_change(vec![Step::Set(vec![first()]), Step::Upsert(entry("h", "a", "openai/m1"))], false, Show::Models, "models=[h:a] ids(a:0) names(h:[0]) set=[\"h\"]")]
#[case::upsert_releases_before_it_admits(vec![Step::Set(vec![first()]), Step::Upsert(entry("g", "a", "openai/m3"))], false, Show::Calls, "admit:g,adaptive:1,invalidated,released:a,admit:g,registered:a,invalidated")]
#[case::upsert_of_an_adaptive_deployment_finalizes(vec![Step::Set(vec![entry("adaptive", "a", "openai/m1")]), Step::Upsert(entry("adaptive", "a", "openai/m3"))], false, Show::Calls, "admit:adaptive,adaptive:1,invalidated,released:a,admit:adaptive,registered:a,adaptive:1,invalidated")]
#[case::upsert_away_from_an_adaptive_deployment_finalizes(vec![Step::Set(vec![entry("adaptive", "a", "openai/m1")]), Step::Upsert(entry("g", "a", "openai/m1"))], false, Show::Calls, "admit:adaptive,adaptive:1,invalidated,released:a,admit:g,registered:a,adaptive:1,invalidated")]
#[case::delete_reindexes_what_follows(vec![Step::Set(vec![first(), second(), entry("h", "c", "openai/m")]), Step::Delete("b")], false, Show::Models, "models=[g:a,h:c] ids(a:0,c:1) names(g:[0],h:[1]) set=[\"g\",\"h\"]")]
#[case::delete_first_of_three(vec![Step::Set(vec![first(), second(), entry("g", "c", "openai/m")]), Step::Delete("a")], false, Show::Models, "models=[g:b,g:c] ids(b:0,c:1) names(g:[0, 1]) set=[\"g\"]")]
#[case::delete_last_of_three(vec![Step::Set(vec![first(), second(), entry("g", "c", "openai/m")]), Step::Delete("c")], false, Show::Models, "models=[g:a,g:b] ids(a:0,b:1) names(g:[0, 1]) set=[\"g\"]")]
#[case::delete_last_of_a_name_leaves_the_set(vec![Step::Set(vec![first(), entry("h", "c", "openai/m")]), Step::Delete("c")], false, Show::Models, "models=[g:a] ids(a:0) names(g:[0]) set=[\"g\"]")]
#[case::delete_reports_the_deployment(vec![Step::Set(vec![first()]), Step::Delete("a")], false, Show::Outcomes, "set:ok;delete:Some(a)")]
#[case::delete_unknown_is_none(vec![Step::Set(vec![first()]), Step::Delete("z")], false, Show::Outcomes, "set:ok;delete:None")]
#[case::delete_unknown_changes_nothing(vec![Step::Set(vec![first()]), Step::Delete("z")], false, Show::Calls, "admit:g,adaptive:1,invalidated")]
#[case::delete_releases_after_publishing(vec![Step::Set(vec![first()]), Step::Delete("a")], false, Show::Calls, "admit:g,adaptive:1,invalidated,invalidated,released:a")]
#[case::delete_duplicate_id_drops_the_entry(vec![Step::Set(vec![first(), entry("g", "a", "openai/m2")]), Step::Delete("a")], false, Show::Models, "models=[g:a] ids() names(g:[0]) set=[\"g\"]")]
#[case::set_runs_the_effects_once(vec![Step::Set(vec![first(), second()])], false, Show::Calls, "admit:g,admit:g,adaptive:2,invalidated")]
#[case::team_models_are_indexed(vec![Step::Set(vec![team_entry("g", "a", "t1", "pub"), team_entry("h", "b", "t1", "pub"), team_entry("g", "c", "t2", "pub")])], false, Show::Teams, "teams=[t1/pub:[0, 1],t2/pub:[2]] public=[pub]")]
#[case::team_delete_reindexes(vec![Step::Set(vec![team_entry("g", "a", "t1", "pub"), team_entry("h", "b", "t1", "pub"), team_entry("g", "c", "t2", "pub")]), Step::Delete("a")], false, Show::Teams, "teams=[t1/pub:[0],t2/pub:[1]] public=[pub]")]
#[case::team_delete_last_drops_the_key(vec![Step::Set(vec![team_entry("g", "a", "t1", "pub1"), team_entry("g", "c", "t2", "pub2")]), Step::Delete("a")], false, Show::Teams, "teams=[t2/pub2:[0]] public=[pub2]")]
#[case::team_needs_an_id(vec![Step::Set(vec![team_entry("g", "a", "", "pub")])], false, Show::Teams, "teams=[] public=[]")]
#[case::team_needs_a_public_name(vec![Step::Set(vec![team_entry("g", "a", "t1", "")])], false, Show::Teams, "teams=[] public=[]")]
#[case::added_team_model_is_indexed(vec![Step::Add(team_entry("g", "a", "t1", "pub"))], false, Show::Teams, "teams=[t1/pub:[0]] public=[pub]")]
#[case::wildcard_goes_to_the_pattern_router(vec![Step::Set(vec![entry("x/*", "a", "openai/*")])], false, Show::Patterns, "defaults=[a] route=[a@openai/y] teams=[] unresolved=[]")]
#[case::wildcard_delete_leaves_the_default_ids(vec![Step::Set(vec![entry("x/*", "a", "openai/*")]), Step::Delete("a")], false, Show::Patterns, "defaults=[a] route=[] teams=[] unresolved=[]")]
#[case::wildcard_with_an_empty_id_is_no_default(vec![Step::Set(vec![entry("x/*", "", "openai/*")])], false, Show::Patterns, "defaults=[] route=[@openai/y] teams=[] unresolved=[]")]
#[case::team_wildcard_goes_to_the_team_router(vec![Step::Set(vec![team_entry("g", "a", "t1", "x/*")])], false, Show::Patterns, "defaults=[] route=[] teams=[t1:a@openai/m] unresolved=[]")]
#[case::team_wildcard_delete_drops_the_router(vec![Step::Set(vec![team_entry("g", "a", "t1", "x/*")]), Step::Delete("a")], false, Show::Patterns, "defaults=[] route=[] teams=[] unresolved=[]")]
#[case::team_name_without_a_wildcard_has_no_router(vec![Step::Set(vec![team_entry("g", "a", "t1", "pub")])], false, Show::Patterns, "defaults=[] route=[] teams=[] unresolved=[]")]
#[case::ignored_failures_are_skipped(vec![Step::Set(vec![first(), entry("unresolved1", "b", "openai/m"), entry("bad1", "c", "openai/m")])], true, Show::Models, "models=[g:a] ids(a:0) names(g:[0]) set=[\"bad1\",\"g\",\"unresolved1\"]")]
#[case::only_unresolved_providers_are_queued(vec![Step::Set(vec![first(), entry("unresolved1", "b", "openai/m"), entry("bad1", "c", "openai/m")])], true, Show::Patterns, "defaults=[] route=[] teams=[] unresolved=[unresolved1]")]
#[case::generated_id_is_the_hash(vec![Step::Inputs(vec![NO_INFO.to_owned()])], false, Show::Outcomes, concat!("inputs:g:\"", "508af05d92d9869c9c853f71a13190cb03fea305b1818633668d09f9958d6d6e", "\":-::-"))]
#[case::declared_ids_are_listed_with_their_duplicates(vec![Step::Inputs(vec![first(), entry("h", "a", "openai/m"), second()])], false, Show::Outcomes, "inputs:g:\"a\":a:a:-|h:\"a\":a:a:-|g:\"b\":b:a:-")]
#[case::each_organization_is_one_input(vec![Step::Inputs(vec![ORGANIZED.to_owned()])], false, Show::Outcomes, concat!("inputs:g:\"", "9eb2e7f5643a3582a5dd4db0cd151e84ebea96fc2631139a6f22e6b1f19d8d1e", "\":-::o1|g:\"", "9eb2e7f5643a3582a5dd4db0cd151e84ebea96fc2631139a6f22e6b1f19d8d1e", "\":-::o2"))]
#[case::numeric_ids_are_declared(vec![Step::Inputs(vec![NUMBERED.to_owned()])], false, Show::Outcomes, "inputs:g:7:7::-")]
#[case::router_level_limits_fill_the_params(vec![Step::Set(vec![TOP_LEVEL_LIMITS.to_owned()])], false, Show::Params, "a:10 20 -")]
#[case::own_params_keep_their_limit(vec![Step::Set(vec![OWN_RPM.to_owned()])], false, Show::Params, "a:5 20 -")]
#[case::null_limits_stay_unset(vec![Step::Set(vec![NULL_LIMITS.to_owned()])], false, Show::Params, "a:- - -")]
#[case::an_added_deployment_gets_the_limits(vec![Step::Add(TOP_LEVEL_LIMITS.to_owned())], false, Show::Params, "a:10 20 -")]
#[case::an_upserted_deployment_gets_the_limits(vec![Step::Set(vec![first()]), Step::Upsert(TOP_LEVEL_LIMITS.to_owned())], false, Show::Params, "a:10 20 -")]
#[case::data_sources_read_the_environment(vec![Step::Set(vec![SOURCES.to_owned()])], false, Show::Params, r#"a:- - [{"type":"t","parameters":{"endpoint":"https://search.example","key":"key-1","index_name":"os.environ/AZURE_KEY"}}]"#)]
#[case::an_added_data_source_reads_the_environment(vec![Step::Add(SOURCES.to_owned())], false, Show::Params, r#"a:- - [{"type":"t","parameters":{"endpoint":"https://search.example","key":"key-1","index_name":"os.environ/AZURE_KEY"}}]"#)]
#[case::unset_variables_read_as_empty(vec![Step::Set(vec![UNSET_SOURCES.to_owned()])], false, Show::Params, r#"a:- - [{"parameters":{"endpoint":"","key":"","index_name":"plain"}}]"#)]
#[case::odd_data_sources_are_left_alone(vec![Step::Set(vec![ODD_SOURCES.to_owned()])], false, Show::Params, r#"a:- - [{"type":"t"},5,{"parameters":{"endpoint":"literal","key":7}},{"parameters":"text"}]"#)]
#[case::set_warns_about_ptu_before_it_builds(vec![Step::Set(vec![ptu("p", r#"{"ptu_count":2}"#), entry("bad1", "c", "openai/m")])], false, Show::Logs, "PTU fields are set on config.yaml deployment(s) p, but PTU cost attribution is disabled, so no flat cost accrues and this traffic is billed per token. Set LITELLM_ENABLE_PTU_COST_ATTRIBUTION=True to enable it")]
#[case::set_drops_the_wildcards_of_the_earlier_list(vec![Step::Set(vec![entry("x/*", "a", "openai/*")]), Step::Set(vec![first()])], false, Show::Patterns, "defaults=[] route=[] teams=[] unresolved=[]")]
#[case::set_drops_the_team_wildcards_of_the_earlier_list(vec![Step::Set(vec![team_entry("g", "a", "t1", "x/*")]), Step::Set(vec![first()])], false, Show::Patterns, "defaults=[] route=[] teams=[] unresolved=[]")]
#[case::an_empty_team_id_still_gets_a_team_router(vec![Step::Set(vec![team_entry("g", "a", "", "x/*")])], false, Show::Patterns, "defaults=[] route=[] teams=[:a@openai/m] unresolved=[]")]
#[tokio::test]
async fn catalog_updates_behaviors(
    #[case] steps: Vec<Step>,
    #[case] ignore: bool,
    #[case] show: Show,
    #[case] expected: &str,
) {
    assert_eq!(observe(steps, ignore, show).await, expected);
}

#[rstest]
#[case::failed_set_keeps_the_published_list(vec![Step::Set(vec![first()]), Step::Set(vec![entry("bad1", "c", "openai/m")])], false, Show::Models, "models=[g:a] ids(a:0) names(g:[0]) set=[\"g\"]")]
#[case::failed_set_names_the_error(vec![Step::Set(vec![first()]), Step::Set(vec![entry("bad1", "c", "openai/m")])], false, Show::Outcomes, "set:ok;set:err(rejected bad1)")]
#[case::failed_set_publishes_nothing(vec![Step::Set(vec![first()]), Step::Set(vec![entry("bad1", "c", "openai/m")])], false, Show::Calls, "admit:g,adaptive:1,invalidated,admit:bad1")]
#[case::unresolved_provider_raises_without_ignore(vec![Step::Set(vec![entry("unresolved1", "b", "openai/m")])], false, Show::Outcomes, "set:err(no provider for unresolved1)")]
#[case::failed_add_keeps_the_catalog(vec![Step::Set(vec![first()]), Step::Add(entry("bad1", "c", "openai/m"))], false, Show::Models, "models=[g:a] ids(a:0) names(g:[0]) set=[\"g\"]")]
#[case::failed_add_raises_even_when_ignoring(vec![Step::Set(vec![first()]), Step::Add(entry("bad1", "c", "openai/m"))], true, Show::Outcomes, "set:ok;add:err(rejected bad1)")]
#[case::failed_upsert_raises_without_ignore(vec![Step::Set(vec![first()]), Step::Upsert(entry("bad1", "a", "openai/m"))], false, Show::Outcomes, "set:ok;upsert:err(rejected bad1)")]
#[case::failed_upsert_keeps_the_previous_deployment(vec![Step::Set(vec![first()]), Step::Upsert(entry("bad1", "a", "openai/m"))], false, Show::Models, "models=[g:a] ids(a:0) names(g:[0]) set=[\"g\"]")]
#[case::failed_upsert_ignored_is_none(vec![Step::Set(vec![first(), second()]), Step::Upsert(entry("bad1", "a", "openai/m"))], true, Show::Outcomes, "set:ok;upsert:None")]
#[case::failed_upsert_ignored_restores_at_the_end(vec![Step::Set(vec![first(), second()]), Step::Upsert(entry("bad1", "a", "openai/m"))], true, Show::Models, "models=[g:b,g:a] ids(a:1,b:0) names(g:[0, 1]) set=[\"g\"]")]
#[case::failed_upsert_ignored_restores_the_only_name(vec![Step::Set(vec![first()]), Step::Upsert(entry("bad1", "a", "openai/m"))], true, Show::Models, "models=[g:a] ids(a:0) names(g:[0]) set=[\"g\"]")]
#[case::failed_upsert_of_a_new_id_changes_nothing(vec![Step::Set(vec![first()]), Step::Upsert(entry("bad1", "z", "openai/m"))], true, Show::Models, "models=[g:a] ids(a:0) names(g:[0]) set=[\"g\"]")]
#[case::failed_upsert_of_a_new_id_publishes_nothing(vec![Step::Set(vec![first()]), Step::Upsert(entry("bad1", "z", "openai/m"))], true, Show::Calls, "admit:g,adaptive:1,invalidated,admit:bad1")]
#[case::failed_restore_keeps_the_previous_deployment(vec![Step::Set(vec![entry("flaky", "a", "openai/m1")]), Step::Upsert(entry("flaky", "a", "openai/m2"))], true, Show::Models, "models=[flaky:a] ids(a:0) names(flaky:[0]) set=[\"flaky\"]")]
#[case::failed_restore_is_none(vec![Step::Set(vec![entry("flaky", "a", "openai/m1")]), Step::Upsert(entry("flaky", "a", "openai/m2"))], true, Show::Outcomes, "set:ok;upsert:None")]
#[case::held_snapshot_survives_a_failed_set(vec![Step::Set(vec![first()]), Step::Hold, Step::Set(vec![entry("bad1", "c", "openai/m")])], false, Show::Held, "models=[g:a] ids(a:0) names(g:[0]) set=[\"g\"]")]
#[case::held_snapshot_survives_a_delete(vec![Step::Set(vec![first()]), Step::Hold, Step::Delete("a")], false, Show::Held, "models=[g:a] ids(a:0) names(g:[0]) set=[\"g\"]")]
#[case::delete_empties_the_published_catalog(vec![Step::Set(vec![first()]), Step::Hold, Step::Delete("a")], false, Show::Models, "models=[] ids() names() set=[]")]
#[case::held_snapshot_survives_an_upsert(vec![Step::Set(vec![first()]), Step::Hold, Step::Upsert(entry("g", "a", "openai/m3"))], false, Show::Held, "models=[g:a] ids(a:0) names(g:[0]) set=[\"g\"]")]
#[case::empty_organization_list_builds_nothing(vec![Step::Set(vec![r#"{"model_name":"g","litellm_params":{"model":"openai/m","organization":[]}}"#.to_owned()])], false, Show::Models, "models=[] ids() names() set=[\"g\"]")]
#[case::ignored_pattern_failures_are_skipped(vec![Step::Set(vec![first(), oversized()])], true, Show::Outcomes, "set:ok")]
#[case::pattern_failures_raise_without_ignore(vec![Step::Set(vec![first()]), Step::Set(vec![second(), oversized()])], false, Show::Models, "models=[g:a] ids(a:0) names(g:[0]) set=[\"g\"]")]
#[tokio::test]
async fn catalog_updates_boundaries(
    #[case] steps: Vec<Step>,
    #[case] ignore: bool,
    #[case] show: Show,
    #[case] expected: &str,
) {
    assert_eq!(observe(steps, ignore, show).await, expected);
}

#[rstest]
#[case::ptu_count(vec![ptu("p", r#"{"ptu_count":2}"#)], false, "p")]
#[case::hourly_rate(vec![ptu("p", r#"{"cost_per_ptu_per_hour":1.5}"#)], false, "p")]
#[case::window_start(vec![ptu("p", r#"{"ptu_effective_from":"2026-01-01T00:00:00Z"}"#)], false, "p")]
#[case::window_end(vec![ptu("p", r#"{"ptu_effective_to":"2026-01-01T00:00:00Z"}"#)], false, "p")]
#[case::null_fields_do_not_count(vec![ptu("p", r#"{"ptu_count":null}"#)], false, "")]
#[case::a_team_alone_is_not_a_declaration(vec![ptu("p", r#"{"team_id":"t"}"#)], false, "")]
#[case::no_ptu_fields(vec![ptu("p", r#"{"id":"a"}"#)], false, "")]
#[case::no_model_info(vec![NO_INFO.to_owned()], false, "")]
#[case::database_models_are_left_out(vec![ptu("p", r#"{"db_model":true,"ptu_count":2}"#)], false, "")]
#[case::config_models_flagged_false_count(vec![ptu("p", r#"{"db_model":false,"ptu_count":2}"#)], false, "p")]
#[case::enabled_attribution_is_silent(vec![ptu("p", r#"{"ptu_count":2}"#)], true, "")]
#[case::names_are_listed_in_order(vec![ptu("p", r#"{"ptu_count":2}"#), ptu("m", r#"{"id":"a"}"#), ptu("q", r#"{"ptu_count":3}"#)], false, "p, q")]
#[test]
fn ptu_declarations_are_warned_about(
    #[case] entries: Vec<String>,
    #[case] attributed: bool,
    #[case] names: &str,
) {
    let (captured, _guard) = capture_logs();
    warn_unattributed_ptu(&parsed(&entries), attributed);
    assert_eq!(captured.messages(), warning(names));
}
