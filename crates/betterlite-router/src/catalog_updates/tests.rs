#![cfg(test)]

use rstest::rstest;
use support::{Show, Step, entry, observe, team_entry};

mod support;

const NO_INFO: &str = r#"{"model_name":"g","litellm_params":{"model":"openai/m"}}"#;
const ORGANIZED: &str =
    r#"{"model_name":"g","litellm_params":{"model":"openai/m","organization":["o1","o2"]}}"#;

fn first() -> String {
    entry("g", "a", "openai/m1")
}

fn second() -> String {
    entry("g", "b", "openai/m2")
}

#[rstest]
#[case::duplicate_names_kept(vec![Step::Set(vec![first(), second()])], false, Show::Models, "models=[g:a,g:b] ids(a:0,b:1) names(g:[0, 1]) set=[g]")]
#[case::duplicate_ids_last_wins(vec![Step::Set(vec![first(), entry("h", "a", "openai/m")])], false, Show::Models, "models=[g:a,h:a] ids(a:1) names(g:[0],h:[1]) set=[g,h]")]
#[case::set_replaces_the_list(vec![Step::Set(vec![first()]), Step::Set(vec![entry("h", "c", "openai/m")])], false, Show::Models, "models=[h:c] ids(c:0) names(h:[0]) set=[h]")]
#[case::empty_name_is_not_indexed(vec![Step::Set(vec![entry("", "a", "openai/m")])], false, Show::Models, "models=[:a] ids(a:0) names() set=[]")]
#[case::add_appends(vec![Step::Set(vec![first()]), Step::Add(second())], false, Show::Models, "models=[g:a,g:b] ids(a:0,b:1) names(g:[0, 1]) set=[g]")]
#[case::add_reports_the_deployment(vec![Step::Set(vec![first()]), Step::Add(second())], false, Show::Outcomes, "set:ok;add:Some(b)")]
#[case::add_known_id_is_none(vec![Step::Set(vec![first()]), Step::Add(entry("h", "a", "openai/m"))], false, Show::Outcomes, "set:ok;add:None")]
#[case::add_known_id_changes_nothing(vec![Step::Set(vec![first()]), Step::Add(entry("h", "a", "openai/m"))], false, Show::Models, "models=[g:a] ids(a:0) names(g:[0]) set=[g]")]
#[case::add_new_name_joins_the_set(vec![Step::Set(vec![first()]), Step::Add(entry("h", "c", "openai/m"))], false, Show::Models, "models=[g:a,h:c] ids(a:0,c:1) names(g:[0],h:[1]) set=[g,h]")]
#[case::add_runs_the_effects_in_order(vec![Step::Add(first())], false, Show::Calls, "admit:g,registered:a,invalidated")]
#[case::upsert_adds_a_new_id(vec![Step::Upsert(first())], false, Show::Outcomes, "upsert:Some(a)")]
#[case::upsert_identical_is_none(vec![Step::Set(vec![first()]), Step::Upsert(first())], false, Show::Outcomes, "set:ok;upsert:None")]
#[case::upsert_identical_runs_no_effects(vec![Step::Set(vec![first()]), Step::Upsert(first())], false, Show::Calls, "admit:g,adaptive:1,invalidated")]
#[case::upsert_changed_params_replaces_at_the_end(vec![Step::Set(vec![first(), second()]), Step::Upsert(entry("g", "a", "openai/m3"))], false, Show::Models, "models=[g:b,g:a] ids(a:1,b:0) names(g:[0, 1]) set=[g]")]
#[case::upsert_changed_name_is_a_change(vec![Step::Set(vec![first()]), Step::Upsert(entry("h", "a", "openai/m1"))], false, Show::Models, "models=[h:a] ids(a:0) names(h:[0]) set=[h]")]
#[case::upsert_releases_before_it_admits(vec![Step::Set(vec![first()]), Step::Upsert(entry("g", "a", "openai/m3"))], false, Show::Calls, "admit:g,adaptive:1,invalidated,released:a,admit:g,registered:a,invalidated")]
#[case::upsert_of_an_adaptive_deployment_finalizes(vec![Step::Set(vec![entry("adaptive", "a", "openai/m1")]), Step::Upsert(entry("adaptive", "a", "openai/m3"))], false, Show::Calls, "admit:adaptive,adaptive:1,invalidated,released:a,admit:adaptive,registered:a,adaptive:1,invalidated")]
#[case::upsert_away_from_an_adaptive_deployment_finalizes(vec![Step::Set(vec![entry("adaptive", "a", "openai/m1")]), Step::Upsert(entry("g", "a", "openai/m1"))], false, Show::Calls, "admit:adaptive,adaptive:1,invalidated,released:a,admit:g,registered:a,adaptive:1,invalidated")]
#[case::delete_reindexes_what_follows(vec![Step::Set(vec![first(), second(), entry("h", "c", "openai/m")]), Step::Delete("b")], false, Show::Models, "models=[g:a,h:c] ids(a:0,c:1) names(g:[0],h:[1]) set=[g,h]")]
#[case::delete_first_of_three(vec![Step::Set(vec![first(), second(), entry("g", "c", "openai/m")]), Step::Delete("a")], false, Show::Models, "models=[g:b,g:c] ids(b:0,c:1) names(g:[0, 1]) set=[g]")]
#[case::delete_last_of_three(vec![Step::Set(vec![first(), second(), entry("g", "c", "openai/m")]), Step::Delete("c")], false, Show::Models, "models=[g:a,g:b] ids(a:0,b:1) names(g:[0, 1]) set=[g]")]
#[case::delete_last_of_a_name_leaves_the_set(vec![Step::Set(vec![first(), entry("h", "c", "openai/m")]), Step::Delete("c")], false, Show::Models, "models=[g:a] ids(a:0) names(g:[0]) set=[g]")]
#[case::delete_reports_the_deployment(vec![Step::Set(vec![first()]), Step::Delete("a")], false, Show::Outcomes, "set:ok;delete:Some(a)")]
#[case::delete_unknown_is_none(vec![Step::Set(vec![first()]), Step::Delete("z")], false, Show::Outcomes, "set:ok;delete:None")]
#[case::delete_unknown_changes_nothing(vec![Step::Set(vec![first()]), Step::Delete("z")], false, Show::Calls, "admit:g,adaptive:1,invalidated")]
#[case::delete_releases_after_publishing(vec![Step::Set(vec![first()]), Step::Delete("a")], false, Show::Calls, "admit:g,adaptive:1,invalidated,invalidated,released:a")]
#[case::delete_duplicate_id_drops_the_entry(vec![Step::Set(vec![first(), entry("g", "a", "openai/m2")]), Step::Delete("a")], false, Show::Models, "models=[g:a] ids() names(g:[0]) set=[g]")]
#[case::set_runs_the_effects_once(vec![Step::Set(vec![first(), second()])], false, Show::Calls, "admit:g,admit:g,adaptive:2,invalidated")]
#[case::team_models_are_indexed(vec![Step::Set(vec![team_entry("g", "a", "t1", "pub"), team_entry("h", "b", "t1", "pub"), team_entry("g", "c", "t2", "pub")])], false, Show::Teams, "teams=[t1/pub:[0, 1],t2/pub:[2]] public=[pub]")]
#[case::team_delete_reindexes(vec![Step::Set(vec![team_entry("g", "a", "t1", "pub"), team_entry("h", "b", "t1", "pub"), team_entry("g", "c", "t2", "pub")]), Step::Delete("a")], false, Show::Teams, "teams=[t1/pub:[0],t2/pub:[1]] public=[pub]")]
#[case::team_delete_last_drops_the_key(vec![Step::Set(vec![team_entry("g", "a", "t1", "pub1"), team_entry("g", "c", "t2", "pub2")]), Step::Delete("a")], false, Show::Teams, "teams=[t2/pub2:[0]] public=[pub2]")]
#[case::team_needs_an_id(vec![Step::Set(vec![team_entry("g", "a", "", "pub")])], false, Show::Teams, "teams=[] public=[]")]
#[case::team_needs_a_public_name(vec![Step::Set(vec![team_entry("g", "a", "t1", "")])], false, Show::Teams, "teams=[] public=[]")]
#[case::added_team_model_is_indexed(vec![Step::Add(team_entry("g", "a", "t1", "pub"))], false, Show::Teams, "teams=[t1/pub:[0]] public=[pub]")]
#[case::wildcard_goes_to_the_pattern_router(vec![Step::Set(vec![entry("x/*", "a", "openai/*")])], false, Show::Patterns, "defaults=[a] route=[a] teams=[] unresolved=[]")]
#[case::wildcard_delete_leaves_the_default_ids(vec![Step::Set(vec![entry("x/*", "a", "openai/*")]), Step::Delete("a")], false, Show::Patterns, "defaults=[a] route=[] teams=[] unresolved=[]")]
#[case::wildcard_with_an_empty_id_is_no_default(vec![Step::Set(vec![entry("x/*", "", "openai/*")])], false, Show::Patterns, "defaults=[] route=[] teams=[] unresolved=[]")]
#[case::team_wildcard_goes_to_the_team_router(vec![Step::Set(vec![team_entry("g", "a", "t1", "x/*")])], false, Show::Patterns, "defaults=[] route=[] teams=[t1:a] unresolved=[]")]
#[case::team_wildcard_delete_drops_the_router(vec![Step::Set(vec![team_entry("g", "a", "t1", "x/*")]), Step::Delete("a")], false, Show::Patterns, "defaults=[] route=[] teams=[] unresolved=[]")]
#[case::team_name_without_a_wildcard_has_no_router(vec![Step::Set(vec![team_entry("g", "a", "t1", "pub")])], false, Show::Patterns, "defaults=[] route=[] teams=[] unresolved=[]")]
#[case::ignored_failures_are_skipped(vec![Step::Set(vec![first(), entry("unresolved1", "b", "openai/m"), entry("bad1", "c", "openai/m")])], true, Show::Models, "models=[g:a] ids(a:0) names(g:[0]) set=[bad1,g,unresolved1]")]
#[case::only_unresolved_providers_are_queued(vec![Step::Set(vec![first(), entry("unresolved1", "b", "openai/m"), entry("bad1", "c", "openai/m")])], true, Show::Patterns, "defaults=[] route=[] teams=[] unresolved=[unresolved1]")]
#[case::generated_id_is_the_hash(vec![Step::Inputs(vec![NO_INFO.to_owned()])], false, Show::Outcomes, concat!("inputs:g:\"", "508af05d92d9869c9c853f71a13190cb03fea305b1818633668d09f9958d6d6e", "\":-:-:-"))]
#[case::declared_ids_are_listed_with_their_duplicates(vec![Step::Inputs(vec![first(), entry("h", "a", "openai/m"), second()])], false, Show::Outcomes, "inputs:g:\"a\":a:a:-|h:\"a\":a:a:-|g:\"b\":b:a:-")]
#[case::each_organization_is_one_input(vec![Step::Inputs(vec![ORGANIZED.to_owned()])], false, Show::Outcomes, concat!("inputs:g:\"", "9eb2e7f5643a3582a5dd4db0cd151e84ebea96fc2631139a6f22e6b1f19d8d1e", "\":-:-:o1|g:\"", "9eb2e7f5643a3582a5dd4db0cd151e84ebea96fc2631139a6f22e6b1f19d8d1e", "\":-:-:o2"))]
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
#[case::failed_set_keeps_the_published_list(vec![Step::Set(vec![first()]), Step::Set(vec![entry("bad1", "c", "openai/m")])], false, Show::Models, "models=[g:a] ids(a:0) names(g:[0]) set=[g]")]
#[case::failed_set_names_the_error(vec![Step::Set(vec![first()]), Step::Set(vec![entry("bad1", "c", "openai/m")])], false, Show::Outcomes, "set:ok;set:err(rejected bad1)")]
#[case::failed_set_publishes_nothing(vec![Step::Set(vec![first()]), Step::Set(vec![entry("bad1", "c", "openai/m")])], false, Show::Calls, "admit:g,adaptive:1,invalidated,admit:bad1")]
#[case::unresolved_provider_raises_without_ignore(vec![Step::Set(vec![entry("unresolved1", "b", "openai/m")])], false, Show::Outcomes, "set:err(no provider for unresolved1)")]
#[case::failed_add_keeps_the_catalog(vec![Step::Set(vec![first()]), Step::Add(entry("bad1", "c", "openai/m"))], false, Show::Models, "models=[g:a] ids(a:0) names(g:[0]) set=[g]")]
#[case::failed_add_raises_even_when_ignoring(vec![Step::Set(vec![first()]), Step::Add(entry("bad1", "c", "openai/m"))], true, Show::Outcomes, "set:ok;add:err(rejected bad1)")]
#[case::failed_upsert_raises_without_ignore(vec![Step::Set(vec![first()]), Step::Upsert(entry("bad1", "a", "openai/m"))], false, Show::Outcomes, "set:ok;upsert:err(rejected bad1)")]
#[case::failed_upsert_keeps_the_previous_deployment(vec![Step::Set(vec![first()]), Step::Upsert(entry("bad1", "a", "openai/m"))], false, Show::Models, "models=[g:a] ids(a:0) names(g:[0]) set=[g]")]
#[case::failed_upsert_ignored_is_none(vec![Step::Set(vec![first(), second()]), Step::Upsert(entry("bad1", "a", "openai/m"))], true, Show::Outcomes, "set:ok;upsert:None")]
#[case::failed_upsert_ignored_restores_at_the_end(vec![Step::Set(vec![first(), second()]), Step::Upsert(entry("bad1", "a", "openai/m"))], true, Show::Models, "models=[g:b,g:a] ids(a:1,b:0) names(g:[0, 1]) set=[g]")]
#[case::failed_upsert_ignored_restores_the_only_name(vec![Step::Set(vec![first()]), Step::Upsert(entry("bad1", "a", "openai/m"))], true, Show::Models, "models=[g:a] ids(a:0) names(g:[0]) set=[g]")]
#[case::failed_upsert_of_a_new_id_changes_nothing(vec![Step::Set(vec![first()]), Step::Upsert(entry("bad1", "z", "openai/m"))], true, Show::Models, "models=[g:a] ids(a:0) names(g:[0]) set=[g]")]
#[case::failed_upsert_of_a_new_id_publishes_nothing(vec![Step::Set(vec![first()]), Step::Upsert(entry("bad1", "z", "openai/m"))], true, Show::Calls, "admit:g,adaptive:1,invalidated,admit:bad1")]
#[case::failed_restore_keeps_the_previous_deployment(vec![Step::Set(vec![entry("flaky", "a", "openai/m1")]), Step::Upsert(entry("flaky", "a", "openai/m2"))], true, Show::Models, "models=[flaky:a] ids(a:0) names(flaky:[0]) set=[flaky]")]
#[case::failed_restore_is_none(vec![Step::Set(vec![entry("flaky", "a", "openai/m1")]), Step::Upsert(entry("flaky", "a", "openai/m2"))], true, Show::Outcomes, "set:ok;upsert:None")]
#[case::held_snapshot_survives_a_failed_set(vec![Step::Set(vec![first()]), Step::Hold, Step::Set(vec![entry("bad1", "c", "openai/m")])], false, Show::Held, "models=[g:a] ids(a:0) names(g:[0]) set=[g]")]
#[case::held_snapshot_survives_a_delete(vec![Step::Set(vec![first()]), Step::Hold, Step::Delete("a")], false, Show::Held, "models=[g:a] ids(a:0) names(g:[0]) set=[g]")]
#[case::delete_empties_the_published_catalog(vec![Step::Set(vec![first()]), Step::Hold, Step::Delete("a")], false, Show::Models, "models=[] ids() names() set=[]")]
#[case::held_snapshot_survives_an_upsert(vec![Step::Set(vec![first()]), Step::Hold, Step::Upsert(entry("g", "a", "openai/m3"))], false, Show::Held, "models=[g:a] ids(a:0) names(g:[0]) set=[g]")]
#[case::empty_organization_list_builds_nothing(vec![Step::Set(vec![r#"{"model_name":"g","litellm_params":{"model":"openai/m","organization":[]}}"#.to_owned()])], false, Show::Models, "models=[] ids() names() set=[g]")]
#[tokio::test]
async fn catalog_updates_boundaries(
    #[case] steps: Vec<Step>,
    #[case] ignore: bool,
    #[case] show: Show,
    #[case] expected: &str,
) {
    assert_eq!(observe(steps, ignore, show).await, expected);
}
