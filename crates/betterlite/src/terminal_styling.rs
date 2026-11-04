use std::io::{self, IsTerminal as _};

const ORANGE: &str = "\x1b[38;2;215;119;87m";
const DIM: &str = "\x1b[2m";
const BOLD: &str = "\x1b[1m";
const GREEN: &str = "\x1b[38;2;78;186;101m";
const BLUE: &str = "\x1b[38;2;177;185;249m";
const GREY: &str = "\x1b[38;2;153;153;153m";
const RESET: &str = "\x1b[0m";

#[must_use]
pub fn supports_color(no_color: bool) -> bool {
    if no_color {
        false
    } else {
        io::stdout().is_terminal()
    }
}

#[must_use]
pub fn color(code: &str, text: &str, supports: bool) -> String {
    if supports {
        format!("{code}{text}{RESET}")
    } else {
        text.to_owned()
    }
}

#[must_use]
pub fn bold(text: &str, supports: bool) -> String {
    if supports {
        format!("{BOLD}{text}{RESET}")
    } else {
        text.to_owned()
    }
}

#[must_use]
pub fn orange(text: &str, supports: bool) -> String {
    if supports {
        format!("{ORANGE}{text}{RESET}")
    } else {
        text.to_owned()
    }
}

#[must_use]
pub fn green(text: &str, supports: bool) -> String {
    if supports {
        format!("{GREEN}{text}{RESET}")
    } else {
        text.to_owned()
    }
}

#[must_use]
pub fn blue(text: &str, supports: bool) -> String {
    if supports {
        format!("{BLUE}{text}{RESET}")
    } else {
        text.to_owned()
    }
}

#[must_use]
pub fn grey(text: &str, supports: bool) -> String {
    if supports {
        format!("{GREY}{text}{RESET}")
    } else {
        text.to_owned()
    }
}

#[must_use]
pub fn dim(text: &str, supports: bool) -> String {
    if supports {
        format!("{DIM}{text}{RESET}")
    } else {
        text.to_owned()
    }
}

mod tests;
