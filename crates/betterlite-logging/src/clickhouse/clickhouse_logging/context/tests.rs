#![cfg(test)]

use super::LensAnalysis;

#[test]
fn the_lens_flag_is_false_outside_a_scope() {
    assert!(!LensAnalysis::default().is_lens_analysis());
}

#[test]
fn the_scope_sets_the_flag_until_it_is_dropped() {
    let lens = LensAnalysis::default();
    let scope = lens.lens_analysis();
    assert!(lens.is_lens_analysis());
    drop(scope);
    assert!(!lens.is_lens_analysis());
}

#[test]
fn a_nested_scope_restores_the_outer_flag() {
    let lens = LensAnalysis::default();
    let outer = lens.lens_analysis();
    let inner = lens.lens_analysis();
    drop(inner);
    assert!(lens.is_lens_analysis());
    drop(outer);
    assert!(!lens.is_lens_analysis());
}

#[test]
fn the_flag_is_restored_after_an_early_return() {
    fn analyze(lens: &LensAnalysis) -> bool {
        let _scope = lens.lens_analysis();
        lens.is_lens_analysis()
    }

    let lens = LensAnalysis::default();
    assert!(analyze(&lens));
    assert!(!lens.is_lens_analysis());
}

#[test]
fn separate_contexts_do_not_share_the_flag() {
    let analysis = LensAnalysis::default();
    let untouched = LensAnalysis::default();
    let _scope = analysis.lens_analysis();
    assert!(analysis.is_lens_analysis());
    assert!(!untouched.is_lens_analysis());
}
