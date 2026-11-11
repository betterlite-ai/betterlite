use console::Key;
use rstest::rstest;

use crate::sso_polling::terminal::{KeyPress, get_key_input};

use super::fixtures::interactive_input;

#[rstest]
#[case::arrow_up(Key::ArrowUp, KeyPress::Up)]
#[case::arrow_down(Key::ArrowDown, KeyPress::Down)]
#[case::enter(Key::Enter, KeyPress::Enter)]
#[case::escape(Key::Escape, KeyPress::Escape)]
#[case::q(Key::Char('q'), KeyPress::Quit)]
fn raw_input_maps_each_selection_key(#[case] key: Key, #[case] expected: KeyPress) {
    let mut input = interactive_input(vec![key]);

    assert_eq!(get_key_input(&mut input), Some(expected));
}

#[rstest]
#[case::upper_q(Key::Char('Q'))]
#[case::other_letter(Key::Char('x'))]
#[case::left_arrow(Key::ArrowLeft)]
#[case::unknown(Key::Unknown)]
#[case::control_c(Key::CtrlC)]
fn raw_input_ignores_every_other_key(#[case] key: Key) {
    let mut input = interactive_input(vec![key]);

    assert_eq!(get_key_input(&mut input), None);
}

#[test]
fn raw_input_returns_none_when_the_terminal_has_no_key() {
    let mut input = interactive_input(Vec::new());

    assert_eq!(get_key_input(&mut input), None);
}
