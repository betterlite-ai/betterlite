use crate::batches::sdk::TimeoutPhases;

// The shared transport applies one total deadline, so every set phase is refused by name
pub(super) fn phase_names(phases: &TimeoutPhases) -> String {
    let mut names = Vec::new();
    if phases.connect.is_some() {
        names.push("connect");
    }
    if phases.read.is_some() {
        names.push("read");
    }
    if phases.write.is_some() {
        names.push("write");
    }
    if phases.pool.is_some() {
        names.push("pool");
    }
    if names.is_empty() {
        String::from("none")
    } else {
        names.join(", ")
    }
}
