use std::collections::BTreeSet;

use crate::outage_alerts::record::{OutageRecord, RegionOutageRecord};

use super::support::{MINOR_SENT, NONE_SENT, deployment_record, region_record};

#[test]
fn record_writes_the_deployment_record_fields() {
    let record = deployment_record(vec![408, 500], MINOR_SENT);
    assert_eq!(
        serde_json::to_string(&record).unwrap(),
        r#"{"model_id":"dep-1","alerts":[408,500],"minor_alert_sent":true,"major_alert_sent":false,"last_updated_at":400.0}"#
    );
}

#[test]
fn record_reads_a_deployment_record_back() {
    let record: OutageRecord = serde_json::from_str(
        r#"{"model_id":"dep-7","alerts":[408,500],"minor_alert_sent":false,"major_alert_sent":true,"last_updated_at":123.5}"#,
    )
    .unwrap();
    assert_eq!(
        record,
        OutageRecord {
            model_id: "dep-7".to_owned(),
            alerts: vec![408, 500],
            minor_alert_sent: false,
            major_alert_sent: true,
            last_updated_at: 123.5,
        }
    );
}

#[test]
fn record_writes_the_region_deployment_ids_as_a_sorted_list() {
    let record = region_record(
        "vertex_aius-east1",
        vec![500],
        &["dep-2", "dep-1", "dep-3"],
        NONE_SENT,
    );
    assert_eq!(
        serde_json::to_string(&record).unwrap(),
        r#"{"provider_region_id":"vertex_aius-east1","alerts":[500],"minor_alert_sent":false,"major_alert_sent":false,"last_updated_at":400.0,"deployment_ids":["dep-1","dep-2","dep-3"]}"#
    );
}

#[test]
fn record_reads_a_deployment_id_list_back_into_a_set() {
    let record: RegionOutageRecord = serde_json::from_str(
        r#"{"provider_region_id":"openaius-east1","alerts":[501],"minor_alert_sent":true,"major_alert_sent":false,"last_updated_at":9.5,"deployment_ids":["dep-2","dep-1","dep-2"]}"#,
    )
    .unwrap();
    assert_eq!(
        record.deployment_ids,
        ["dep-1".to_owned(), "dep-2".to_owned()]
            .into_iter()
            .collect::<BTreeSet<_>>()
    );
}

#[test]
fn record_round_trips_the_region_record() {
    let record = region_record(
        "vertex_aius-east1",
        vec![408, 500],
        &["dep-2", "dep-1"],
        MINOR_SENT,
    );
    let text = serde_json::to_string(&record).unwrap();
    assert_eq!(
        serde_json::from_str::<RegionOutageRecord>(&text).unwrap(),
        record
    );
}
