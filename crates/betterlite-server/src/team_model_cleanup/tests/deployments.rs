use crate::team_model_cleanup::deployments::{
    DeploymentRow, team_deployments, team_public_model_names,
};

use super::support::{FakeStore, deployment};

fn store(deployments: Vec<DeploymentRow>) -> FakeStore {
    FakeStore::new("t1", None, Vec::new(), deployments)
}

#[tokio::test]
async fn team_deployments_keeps_only_the_team_prefix_and_team_id() {
    let cache = store(vec![
        deployment("model_name_t1_a", r#"{"team_id":"t1"}"#),
        deployment("model_name_t1_b", r#"{"team_id":"t2"}"#),
        deployment("model_name_t10_c", r#"{"team_id":"t1"}"#),
        deployment("model_name_t1_d", r#"{"team_id":1}"#),
        deployment("model_name_t1_e", "null"),
        deployment("model_name_t1_f", r#"{"team_id":"t1"}"#),
    ]);
    let found = team_deployments("t1", &cache).await.unwrap();
    let found_names: Vec<&str> = found.iter().map(|row| row.model_name.as_str()).collect();
    assert_eq!(found_names, vec!["model_name_t1_a", "model_name_t1_f"]);
}

#[tokio::test]
async fn team_public_model_names_keeps_the_truthy_text_names() {
    let cache = store(vec![
        deployment(
            "model_name_t1_a",
            r#"{"team_id":"t1","team_public_model_name":"pub"}"#,
        ),
        deployment(
            "model_name_t1_b",
            r#"{"team_id":"t1","team_public_model_name":"pub"}"#,
        ),
        deployment(
            "model_name_t1_c",
            r#"{"team_id":"t1","team_public_model_name":""}"#,
        ),
        deployment("model_name_t1_d", r#"{"team_id":"t1"}"#),
        deployment(
            "model_name_t1_e",
            r#"{"team_id":"t1","team_public_model_name":7}"#,
        ),
        deployment(
            "model_name_t1_f",
            r#"{"team_id":"t1","team_public_model_name":"other"}"#,
        ),
    ]);
    let found = team_public_model_names("t1", &cache).await.unwrap();
    let found_names: Vec<&str> = found.iter().map(String::as_str).collect();
    assert_eq!(found_names, vec!["other", "pub"]);
}

#[tokio::test]
async fn no_deployments_means_no_public_names() {
    let cache = store(Vec::new());
    let found = team_public_model_names("t1", &cache).await.unwrap();
    assert!(found.is_empty());
}
