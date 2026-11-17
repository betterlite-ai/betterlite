#![cfg(test)]

use std::env;
use std::fs;

use crate::dispatch::sandbox::write_settings;

const BEFORE_HOOKS: &str = r#"{"sandbox":{"enabled":true,"autoAllowBashIfSandboxed":true,"filesystem":{"allowWrite":["/w","/g","/h/.cargo/registry"]},"network":{"allowedDomains":["*"]}},"permissions":{"deny":["Bash(env-uGATE_DISPATCHED:*)","Bash(unsetGATE_DISPATCHED:*)","Bash(GATE_DISPATCHED=:*)","Bash(~/.cargo/bin/cargo:*)","Bash($HOME/.cargo/bin/cargo:*)","Bash(/h/.cargo/bin/cargo:*)","Bash(cargogaterun:*)","Bash(cargogatepreflight:*)","Bash(cargogatehook:*)","Bash(cargofmt:*)"]},"attribution":{"commit":"","pr":""}"#;

#[test]
fn settings_sandbox_bash_and_give_only_a_worker_the_stop_hook() {
    let cases = [
        ("reviewer", None, ""),
        (
            "worker",
            Some("/gate"),
            r#","hooks":{"Stop":[{"hooks":[{"type":"command","command":"cd\"$CLAUDE_PROJECT_DIR\"&&'/gate'hook--reportjson-schema","timeout":300}]}]}"#,
        ),
    ];
    for (seat, stop_gate, hooks) in cases {
        let path = env::temp_dir().join(format!("betterlite-gate-sandbox-settings-{seat}.json"));
        write_settings(
            &path,
            "/w",
            "/h",
            &["/g".to_owned(), "/h/.cargo/registry".to_owned()],
            stop_gate,
        )
        .unwrap();
        let text = fs::read_to_string(&path).unwrap();
        let compact: String = text.split_whitespace().collect();
        assert_eq!(compact, format!("{BEFORE_HOOKS}{hooks}}}"), "{seat}");
    }
}
