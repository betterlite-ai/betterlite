#![cfg(test)]

use std::env;
use std::fs;
use std::process;

use crate::backlog::read_backlog;

fn entry(id: &str) -> String {
    format!(
        "id: {id}\nmilestone: M0 foundation\ntitle: Probe\nstatus: todo\ndepends_on: []\nfile: null\ncovers: []\n"
    )
}

#[test]
fn entries_load_in_id_order_and_misnamed_files_are_problems() {
    let development = env::temp_dir().join(format!("betterlite-gate-backlog-{}", process::id()));
    let folder = development.join("backlog");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("B1000.yaml"), entry("B1000")).unwrap();
    fs::write(folder.join("B010.yaml"), entry("B010")).unwrap();
    fs::write(folder.join("B009.yaml"), entry("B099")).unwrap();
    fs::write(folder.join("notes.txt"), "not an entry").unwrap();
    let (backlog, problems) = read_backlog(&development).unwrap();
    let ids: Vec<&str> = backlog.tasks.iter().map(|task| task.id.as_str()).collect();
    assert_eq!(ids, ["B010", "B099", "B1000"]);
    assert_eq!(problems.len(), 2);
    assert!(
        problems
            .iter()
            .any(|problem| problem.contains("holds B099, so it must be named B099.yaml"))
    );
    assert!(
        problems
            .iter()
            .any(|problem| problem.contains("backlog entries are <id>.yaml files"))
    );
    fs::remove_dir_all(&development).unwrap();
}
