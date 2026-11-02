use crate::loader::EnvironmentLookup;

// LiteLLM checks TERM_PROGRAM before every terminal-specific variable, WezTerm included
pub(crate) fn resolve(lookup: &dyn EnvironmentLookup) -> String {
    term_program(lookup)
        .or_else(|| wezterm(lookup))
        .or_else(|| iterm_present(lookup).then_some("iTerm.app".to_owned()))
        .or_else(|| env_present(lookup, "TERM_SESSION_ID").then_some("Apple_Terminal".to_owned()))
        .or_else(|| kitty_present(lookup).then_some("kitty".to_owned()))
        .or_else(|| {
            let term_is_alacritty = lookup.lookup("TERM").as_deref() == Some("alacritty");
            (env_present(lookup, "ALACRITTY_SOCKET") || term_is_alacritty)
                .then_some("Alacritty".to_owned())
        })
        .or_else(|| konsole(lookup))
        .or_else(|| {
            env_present(lookup, "GNOME_TERMINAL_SCREEN").then_some("gnome-terminal".to_owned())
        })
        .or_else(|| vte(lookup))
        .or_else(|| env_present(lookup, "WT_SESSION").then_some("WindowsTerminal".to_owned()))
        .or_else(|| {
            let term = lookup.lookup("TERM").filter(|value| !value.is_empty())?;
            Some(sanitized_or(&term, "unknown"))
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

fn iterm_present(lookup: &dyn EnvironmentLookup) -> bool {
    env_present(lookup, "ITERM_SESSION_ID")
        || env_present(lookup, "ITERM_PROFILE")
        || env_present(lookup, "ITERM_PROFILE_NAME")
}

fn kitty_present(lookup: &dyn EnvironmentLookup) -> bool {
    let term_names_kitty = lookup
        .lookup("TERM")
        .is_some_and(|term| term.contains("kitty"));
    env_present(lookup, "KITTY_WINDOW_ID") || term_names_kitty
}

fn env_present(lookup: &dyn EnvironmentLookup, name: &str) -> bool {
    let Some(value) = lookup.lookup(name) else {
        return false;
    };
    !value.is_empty()
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

fn wezterm(lookup: &dyn EnvironmentLookup) -> Option<String> {
    let version = lookup.lookup("WEZTERM_VERSION")?;
    if version.is_empty() {
        Some(sanitized_or("WezTerm", "WezTerm"))
    } else {
        Some(sanitized_or(&format!("WezTerm/{version}"), "WezTerm"))
    }
}

fn konsole(lookup: &dyn EnvironmentLookup) -> Option<String> {
    let version = lookup.lookup("KONSOLE_VERSION")?;
    if version.is_empty() {
        Some(sanitized_or("Konsole", "Konsole"))
    } else {
        Some(sanitized_or(&format!("Konsole/{version}"), "Konsole"))
    }
}

fn vte(lookup: &dyn EnvironmentLookup) -> Option<String> {
    let version = lookup.lookup("VTE_VERSION")?;
    if version.is_empty() {
        Some(sanitized_or("VTE", "VTE"))
    } else {
        Some(sanitized_or(&format!("VTE/{version}"), "VTE"))
    }
}

mod tests;
