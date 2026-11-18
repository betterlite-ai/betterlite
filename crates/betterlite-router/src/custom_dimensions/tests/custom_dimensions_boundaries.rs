use std::collections::BTreeMap;

use betterlite_config::sections::custom_dimensions as section;
use betterlite_config::sections::custom_dimensions::pattern::PatternProblem;
use rstest::rstest;

use super::{class_repeat, counted, dim, filled, keyworded, long_name, many, named, run, weighted};
use crate::custom_dimensions::{
    DimensionsError, MAX_CUSTOM_DIMENSIONS, MAX_CUSTOM_DIMENSIONS_WORK,
};

fn fifteen_then(last: usize) -> Vec<usize> {
    let mut lengths = vec![256; 15];
    lengths.push(last);
    lengths
}

#[rstest]
#[case::name_of_sixty_four(vec![long_name(64)], Ok(0))]
#[case::name_of_sixty_five(
    vec![long_name(65)],
    Err(DimensionsError::Dimension(section::DimensionProblem::Name))
)]
#[case::name_with_digits_and_underscore(vec![named("Abc_123")], Ok(0))]
#[case::name_starting_with_an_underscore(
    vec![named("_a")],
    Err(DimensionsError::Dimension(section::DimensionProblem::Name))
)]
#[case::name_with_a_dash(
    vec![named("a-b")],
    Err(DimensionsError::Dimension(section::DimensionProblem::Name))
)]
#[case::an_empty_name(
    vec![named("")],
    Err(DimensionsError::Dimension(section::DimensionProblem::Name))
)]
#[case::weight_of_zero(
    vec![weighted(0.0)],
    Err(DimensionsError::Dimension(section::DimensionProblem::Weight))
)]
#[case::weight_of_one(vec![weighted(1.0)], Ok(0))]
#[case::weight_above_one(
    vec![weighted(1.5)],
    Err(DimensionsError::Dimension(section::DimensionProblem::Weight))
)]
#[case::weight_of_not_a_number(
    vec![weighted(f64::NAN)],
    Err(DimensionsError::Dimension(section::DimensionProblem::Weight))
)]
#[case::weight_of_infinity(
    vec![weighted(f64::INFINITY)],
    Err(DimensionsError::Dimension(section::DimensionProblem::Weight))
)]
#[case::keyword_of_one_character(vec![keyworded(1)], Ok(0))]
#[case::keyword_of_two_hundred_fifty_six_characters(vec![keyworded(256)], Ok(0))]
#[case::keyword_of_two_hundred_fifty_seven_characters(
    vec![keyworded(257)],
    Err(DimensionsError::Dimension(section::DimensionProblem::MatcherLength))
)]
#[case::thirty_two_matchers(vec![counted(32)], Ok(0))]
#[case::thirty_three_matchers(
    vec![counted(33)],
    Err(DimensionsError::Dimension(section::DimensionProblem::MatcherLimits))
)]
#[case::sixteen_patterns_of_two_hundred_fifty_six_characters(
    vec![filled("d1", vec![256; 16])],
    Ok(4112)
)]
#[case::one_character_over_four_thousand_ninety_six(
    vec![filled("d1", vec![241; 17])],
    Err(DimensionsError::Dimension(section::DimensionProblem::MatcherLimits))
)]
#[case::sixteen_dimensions(many(16), Ok(0))]
#[case::seventeen_dimensions(
    many(17),
    Err(DimensionsError::Count {
        count: 17,
        limit: MAX_CUSTOM_DIMENSIONS,
    })
)]
#[case::work_of_exactly_the_router_limit(
    vec![filled("d1", vec![256; 16]), filled("d2", fifteen_then(224))],
    Ok(8192)
)]
#[case::one_step_over_the_router_limit(
    vec![filled("d1", vec![256; 16]), filled("d2", fifteen_then(225))],
    Err(DimensionsError::Work {
        work: 8193,
        limit: MAX_CUSTOM_DIMENSIONS_WORK,
    })
)]
#[case::repeat_of_exactly_sixty_four(vec![dim("domain", &[], &["a{64}"])], Ok(67))]
#[case::repeat_of_sixty_five(
    vec![dim("domain", &[], &["a{65}"])],
    Err(DimensionsError::Dimension(section::DimensionProblem::Pattern {
        pattern: "a{65}".to_owned(),
        problem: PatternProblem::Repeat,
    }))
)]
#[case::reversed_repeat_bounds(
    vec![dim("domain", &[], &["a{2,1}"])],
    Err(DimensionsError::Dimension(section::DimensionProblem::Pattern {
        pattern: "a{2,1}".to_owned(),
        problem: PatternProblem::Syntax,
    }))
)]
#[case::class_repeat_just_under_the_work_limit(vec![class_repeat(30)], Ok(1987))]
#[case::class_repeat_just_over_the_work_limit(
    vec![class_repeat(31)],
    Err(DimensionsError::Dimension(section::DimensionProblem::Pattern {
        pattern: class_repeat(31).patterns[0].clone(),
        problem: PatternProblem::Work,
    }))
)]
#[case::a_blank_keyword(
    vec![dim("domain", &[" "], &[])],
    Err(DimensionsError::Dimension(section::DimensionProblem::BlankMatchers))
)]
#[case::an_empty_matcher(
    vec![dim("domain", &[""], &[])],
    Err(DimensionsError::Dimension(section::DimensionProblem::MatcherLength))
)]
fn custom_dimensions_boundaries(
    #[case] declared: Vec<section::Config>,
    #[case] expected: Result<u64, DimensionsError>,
) {
    assert_eq!(run(&declared, "heuristic", &BTreeMap::new()), expected);
}
