#![cfg(test)]

use clap::error::ErrorKind;
use clap::{CommandFactory as _, Parser as _};

use crate::commands::configure::args::{
    CONFIGURE_ABOUT, CONFIGURE_CLAUDE_ABOUT, DEFAULT_MODEL_HELP, GROUP_API_KEY_HELP,
    RECONFIGURE_ABOUT, TARGET_API_KEY_HELP, UNCONFIGURE_ABOUT, UNCONFIGURE_CLAUDE_ABOUT,
};
use crate::commands::configure::args::{ConfigureTarget, ReconfigureTarget};
use crate::commands::configure::test_support::{TestCli, TestCommand};

fn option_help(command: &clap::Command, name: &str) -> String {
    command
        .get_arguments()
        .find(|argument| argument.get_long() == Some(name))
        .unwrap()
        .get_help()
        .unwrap()
        .to_string()
}

fn about_of(command: &clap::Command) -> String {
    command.get_about().unwrap().to_string()
}

#[test]
fn command_help_carries_the_litellm_texts() {
    let command = TestCli::command();
    let configure = command.find_subcommand("configure").unwrap();
    let reconfigure = command.find_subcommand("reconfigure").unwrap();
    let unconfigure = command.find_subcommand("unconfigure").unwrap();
    let claude = configure.find_subcommand("claude").unwrap();
    assert_eq!(about_of(configure), CONFIGURE_ABOUT);
    assert_eq!(about_of(reconfigure), RECONFIGURE_ABOUT);
    assert_eq!(about_of(unconfigure), UNCONFIGURE_ABOUT);
    assert_eq!(about_of(claude), CONFIGURE_CLAUDE_ABOUT);
    assert_eq!(
        about_of(unconfigure.find_subcommand("claude").unwrap()),
        UNCONFIGURE_CLAUDE_ABOUT
    );
    assert_eq!(option_help(configure, "api-key"), GROUP_API_KEY_HELP);
    assert_eq!(option_help(claude, "api-key"), TARGET_API_KEY_HELP);
    assert_eq!(option_help(claude, "default-model"), DEFAULT_MODEL_HELP);
}

#[test]
fn only_claude_subcommands_offer_default_model() {
    let configure_claude =
        TestCli::try_parse_from(["lite", "configure", "claude", "--default-model"]).unwrap();
    let TestCommand::Configure(args) = configure_claude.command else {
        panic!("expected the configure command");
    };
    let Some(ConfigureTarget::Claude { default_model, .. }) = args.command else {
        panic!("expected the claude subcommand");
    };
    assert!(default_model);
    let reconfigure_claude =
        TestCli::try_parse_from(["lite", "reconfigure", "claude", "--default-model"]).unwrap();
    let TestCommand::Reconfigure(reconfigure_args) = reconfigure_claude.command else {
        panic!("expected the reconfigure command");
    };
    let Some(ReconfigureTarget::Claude {
        default_model: reconfigure_default_model,
        ..
    }) = reconfigure_args.command
    else {
        panic!("expected the claude subcommand");
    };
    assert!(reconfigure_default_model);
    let configure_codex =
        TestCli::try_parse_from(["lite", "configure", "codex", "--default-model"])
            .err()
            .unwrap();
    assert_eq!(configure_codex.kind(), ErrorKind::UnknownArgument);
    let reconfigure_codex =
        TestCli::try_parse_from(["lite", "reconfigure", "codex", "--default-model"])
            .err()
            .unwrap();
    assert_eq!(reconfigure_codex.kind(), ErrorKind::UnknownArgument);
}
