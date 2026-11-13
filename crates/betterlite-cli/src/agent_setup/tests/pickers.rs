#[test]
fn pickers_targets_route_message_and_defaults() {
    let mut prompter =
        ScriptedPrompter::new(vec![Reply::Targets(vec![Target::Claude, Target::Codex])]);
    let picked = pick_targets(&mut prompter, &[Target::Claude, Target::Codex], false).unwrap();
    assert_eq!(picked, vec![Target::Claude, Target::Codex]);
    assert_eq!(
        prompter.calls,
        vec![Call::Targets {
            message: TARGET_ROUTE_MESSAGE.to_owned(),
            defaults: vec![Target::Claude, Target::Codex],
            on_empty: EMPTY_TARGETS_MESSAGE.to_owned(),
        }]
    );
}

#[test]
fn pickers_targets_edit_message() {
    let mut prompter = ScriptedPrompter::new(vec![Reply::Targets(vec![Target::Claude])]);
    let picked = pick_targets(&mut prompter, &[Target::Claude], true).unwrap();
    assert_eq!(picked, vec![Target::Claude]);
    let Call::Targets { message, .. } = &prompter.calls[0] else {
        panic!("expected a target checkbox");
    };
    assert_eq!(message, TARGET_EDIT_MESSAGE);
}

#[test]
fn pickers_targets_use_the_given_defaults() {
    let mut prompter = ScriptedPrompter::new(vec![Reply::Targets(vec![Target::Codex])]);
    let picked = pick_targets(&mut prompter, &[Target::Codex], false).unwrap();
    assert_eq!(picked, vec![Target::Codex]);
    let Call::Targets { defaults, .. } = &prompter.calls[0] else {
        panic!("expected a target checkbox");
    };
    assert_eq!(defaults, &vec![Target::Codex]);
}

#[test]
fn pickers_targets_ask_again_on_an_empty_answer() {
    let mut prompter = ScriptedPrompter::new(vec![
        Reply::Targets(Vec::new()),
        Reply::Targets(vec![Target::Claude]),
    ]);
    let picked = pick_targets(&mut prompter, &[Target::Claude, Target::Codex], false).unwrap();
    assert_eq!(picked, vec![Target::Claude]);
    assert_eq!(prompter.calls.len(), 2);
    let Call::Targets { on_empty, .. } = &prompter.calls[1] else {
        panic!("expected a second target checkbox");
    };
    assert_eq!(on_empty, EMPTY_TARGETS_MESSAGE);
}

#[test]
fn pickers_targets_return_claude_codex_order() {
    let mut prompter =
        ScriptedPrompter::new(vec![Reply::Targets(vec![Target::Codex, Target::Claude])]);
    let picked = pick_targets(&mut prompter, &[Target::Claude, Target::Codex], false).unwrap();
    assert_eq!(picked, vec![Target::Claude, Target::Codex]);
}

#[test]
fn pickers_claude_message_choices_and_preselection() {
    let names = vec!["sonnet".to_owned(), "haiku".to_owned()];
    let mut prompter = ScriptedPrompter::new(vec![Reply::Model("haiku".to_owned())]);
    let picked = pick_claude_model(&mut prompter, &names, Some("haiku")).unwrap();
    assert_eq!(picked.as_deref(), Some("haiku"));
    assert_eq!(
        prompter.calls,
        vec![Call::Model {
            message: CLAUDE_MODEL_MESSAGE.to_owned(),
            choices: vec![
                KEEP_CLAUDE_DEFAULT.to_owned(),
                "sonnet".to_owned(),
                "haiku".to_owned(),
            ],
            selected: 2,
        }]
    );
}

#[test]
fn pickers_claude_default_not_served_selects_keep() {
    let names = vec!["sonnet".to_owned()];
    let mut prompter = ScriptedPrompter::new(vec![Reply::Model("sonnet".to_owned())]);
    let picked = pick_claude_model(&mut prompter, &names, Some("gone")).unwrap();
    assert_eq!(picked.as_deref(), Some("sonnet"));
    let Call::Model { selected, .. } = &prompter.calls[0] else {
        panic!("expected a model picker");
    };
    assert_eq!(*selected, 0);
}

#[test]
fn pickers_claude_keep_row_returns_none() {
    let names = vec!["sonnet".to_owned()];
    let mut prompter = ScriptedPrompter::new(vec![Reply::Model(KEEP_CLAUDE_DEFAULT.to_owned())]);
    let picked = pick_claude_model(&mut prompter, &names, None).unwrap();
    assert!(picked.is_none());
}

#[test]
fn pickers_codex_message_and_preselection() {
    let ids = vec!["codex-a".to_owned(), "codex-b".to_owned()];
    let mut prompter = ScriptedPrompter::new(vec![Reply::Model("codex-b".to_owned())]);
    let picked = pick_codex_model(&mut prompter, &ids, Some("codex-b")).unwrap();
    assert_eq!(picked, "codex-b");
    assert_eq!(
        prompter.calls,
        vec![Call::Model {
            message: CODEX_MODEL_MESSAGE.to_owned(),
            choices: ids,
            selected: 1,
        }]
    );
}

#[test]
fn pickers_codex_default_not_listed_selects_first() {
    let ids = vec!["codex-a".to_owned(), "codex-b".to_owned()];
    let mut prompter = ScriptedPrompter::new(vec![Reply::Model("codex-a".to_owned())]);
    let picked = pick_codex_model(&mut prompter, &ids, Some("gone")).unwrap();
    assert_eq!(picked, "codex-a");
    let Call::Model { selected, .. } = &prompter.calls[0] else {
        panic!("expected a model picker");
    };
    assert_eq!(*selected, 0);
}

#[test]
fn pickers_model_option_help_names_the_role() {
    assert!(MODEL_OPTION_HELP.contains(STARTING_MODEL_ROLE));
    assert!(MODEL_OPTION_HELP.contains("--default-model"));
    assert!(MODEL_OPTION_HELP.contains("lite autoroute start"));
    assert!(MODEL_OPTION_HELP.contains("/v1/models"));
}
