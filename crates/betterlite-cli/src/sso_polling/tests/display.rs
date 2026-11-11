use rust_decimal::Decimal;

use crate::credentials::contracts::CliTeam;
use crate::sso_polling::CLEAR_SCREEN;
use crate::sso_polling::display::{SELECT_TEAM_HEADER, display_team_selection};

use super::fixtures::Echo;

fn sample_teams() -> Vec<CliTeam> {
    vec![
        CliTeam {
            team_id: Some(String::from("team-1")),
            team_alias: Some(String::from("Alpha")),
            models: vec![
                String::from("gpt-4o"),
                String::from("claude"),
                String::from("gemini"),
                String::from("mistral"),
            ],
            max_budget: Some(Decimal::new(10050, 2)),
        },
        CliTeam {
            team_id: Some(String::from("team-2")),
            team_alias: None,
            models: Vec::new(),
            max_budget: None,
        },
    ]
}

#[test]
fn display_renders_aliases_models_and_budgets_with_the_selected_marker() {
    let echo = Echo::default();
    let sink = echo.sink();

    display_team_selection(&sample_teams(), 0, false, &sink);

    let messages = echo.recorded();
    assert_eq!(messages[0], SELECT_TEAM_HEADER);
    assert_eq!(messages[1], "> Alpha (team-1)");
    assert_eq!(messages[2], "   Models: gpt-4o, claude, gemini (+1 more)");
    assert_eq!(messages[3], "   Budget: $100.50");
    assert_eq!(messages[4], "");
    assert_eq!(messages[5], "  N/A (team-2)");
    assert_eq!(messages[6], "   Models: All models");
    assert_eq!(messages[7], "   Budget: Unlimited");
    assert_eq!(messages[8], "");
    assert_eq!(messages.len(), 9);
}

#[test]
fn display_lists_exactly_three_models_without_a_remainder() {
    let echo = Echo::default();
    let sink = echo.sink();
    let exactly_three = vec![CliTeam {
        team_id: Some(String::from("team-1")),
        team_alias: Some(String::from("Alpha")),
        models: vec![
            String::from("gpt-4o"),
            String::from("claude"),
            String::from("gemini"),
        ],
        max_budget: None,
    }];

    display_team_selection(&exactly_three, 0, false, &sink);

    assert_eq!(echo.recorded()[2], "   Models: gpt-4o, claude, gemini");
}

#[test]
fn display_emits_no_escape_codes_when_styling_is_off() {
    let echo = Echo::default();
    let sink = echo.sink();

    display_team_selection(&sample_teams(), 0, false, &sink);

    assert!(
        echo.recorded()
            .iter()
            .all(|message| !message.contains('\u{1b}'))
    );
}

#[test]
fn display_clears_the_screen_and_styles_the_selected_team() {
    let echo = Echo::default();
    let sink = echo.sink();

    display_team_selection(&sample_teams(), 1, true, &sink);

    let messages = echo.recorded();
    assert_eq!(messages[0], CLEAR_SCREEN);
    assert_eq!(messages[1], SELECT_TEAM_HEADER);
    assert!(messages[2].starts_with("  "));
    assert!(messages[2].contains('\u{1b}'));
    assert!(messages[6].starts_with("> "));
    assert!(messages[6].contains('\u{1b}'));
    assert!(messages[6].contains("N/A"));
}

#[test]
fn display_highlights_nothing_when_the_index_is_past_the_end() {
    let echo = Echo::default();
    let sink = echo.sink();

    display_team_selection(&sample_teams(), 9, false, &sink);

    assert!(
        echo.recorded()
            .iter()
            .skip(1)
            .all(|message| !message.starts_with("> "))
    );
}
