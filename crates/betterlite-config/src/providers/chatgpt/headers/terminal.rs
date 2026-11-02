use crate::loader::EnvironmentLookup;

// LiteLLM checks TERM_PROGRAM before every terminal-specific variable, WezTerm included
pub(crate) fn resolve(lookup: &dyn EnvironmentLookup) -> String {
    term_program(lookup)
        .or_else(|| versioned_terminal(lookup, "WEZTERM_VERSION", "WezTerm"))
        .or_else(|| iterm_present(lookup).then_some("iTerm.app".to_owned()))
        .or_else(|| {
            lookup
                .lookup("TERM_SESSION_ID")
                .is_some_and(|value| !value.is_empty())
                .then_some("Apple_Terminal".to_owned())
        })
        .or_else(|| kitty_present(lookup).then_some("kitty".to_owned()))
        .or_else(|| {
            let socket_present = lookup
                .lookup("ALACRITTY_SOCKET")
                .is_some_and(|value| !value.is_empty());
            let term_is_alacritty = lookup.lookup("TERM").as_deref() == Some("alacritty");
            (socket_present || term_is_alacritty).then_some("Alacritty".to_owned())
        })
        .or_else(|| versioned_terminal(lookup, "KONSOLE_VERSION", "Konsole"))
        .or_else(|| {
            lookup
                .lookup("GNOME_TERMINAL_SCREEN")
                .is_some_and(|value| !value.is_empty())
                .then_some("gnome-terminal".to_owned())
        })
        .or_else(|| versioned_terminal(lookup, "VTE_VERSION", "VTE"))
        .or_else(|| {
            lookup
                .lookup("WT_SESSION")
                .is_some_and(|value| !value.is_empty())
                .then_some("WindowsTerminal".to_owned())
        })
        .or_else(|| {
            let term = lookup.lookup("TERM").filter(|value| !value.is_empty())?;
            Some(sanitized_or(&term, "unknown"))
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

fn iterm_present(lookup: &dyn EnvironmentLookup) -> bool {
    for name in ["ITERM_SESSION_ID", "ITERM_PROFILE", "ITERM_PROFILE_NAME"] {
        if lookup.lookup(name).is_some_and(|value| !value.is_empty()) {
            return true;
        }
    }
    false
}

fn kitty_present(lookup: &dyn EnvironmentLookup) -> bool {
    let window_id_present = lookup
        .lookup("KITTY_WINDOW_ID")
        .is_some_and(|value| !value.is_empty());
    let term_names_kitty = lookup
        .lookup("TERM")
        .is_some_and(|term| term.contains("kitty"));
    window_id_present || term_names_kitty
}

fn sanitize_token(value: &str) -> String {
    if value.is_empty() {
        String::new()
    } else {
        value
            .chars()
            .map(|letter| {
                if letter.is_alphanumeric() || "-_./".contains(letter) {
                    letter
                } else {
                    '_'
                }
            })
            .collect()
    }
}

fn sanitized_or(token: &str, fallback: &str) -> String {
    let sanitized = sanitize_token(token);
    if sanitized.is_empty() {
        fallback.to_owned()
    } else {
        sanitized
    }
}

fn term_program(lookup: &dyn EnvironmentLookup) -> Option<String> {
    let program = lookup
        .lookup("TERM_PROGRAM")
        .filter(|value| !value.is_empty())?;
    let maybe_version = lookup
        .lookup("TERM_PROGRAM_VERSION")
        .filter(|value| !value.is_empty());
    let token = match maybe_version {
        Some(found_version) => format!("{program}/{found_version}"),
        None => program,
    };
    Some(sanitized_or(&token, "unknown"))
}

// WezTerm, Konsole and VTE each select on their own version variable, keeping the bare name when it's present but empty
fn versioned_terminal(
    lookup: &dyn EnvironmentLookup,
    version_var: &str,
    name: &str,
) -> Option<String> {
    let version = lookup.lookup(version_var)?;
    if version.is_empty() {
        Some(sanitized_or(name, name))
    } else {
        Some(sanitized_or(&format!("{name}/{version}"), name))
    }
}

mod tests;
