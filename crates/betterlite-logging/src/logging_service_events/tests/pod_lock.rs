use std::sync::{Arc, Mutex};

use betterlite_db::queues::pod_lock::{PodLockCallType, PodLockEvent, PodLockEventSink as _};
use chrono::{DateTime, FixedOffset};
use rstest::rstest;

use crate::logging_service_events::pipeline::{PipelineConfig, ServicePipeline};
use crate::logging_service_events::sink::{ServiceEventError, ServiceEventSink};
use crate::logging_service_events::types::{
    ServiceEvent, ServiceEventMetadata, ServiceLoggerPayload, ServiceType,
};
use crate::logging_success::ports::SuccessClock;
use crate::registry::CallbackRegistry;
use crate::worker::WorkerSettings;

struct FixedClock(DateTime<FixedOffset>);

impl SuccessClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        self.0
    }
}

#[derive(Default)]
struct RecordingSink(Mutex<Vec<ServiceEvent>>);

impl ServiceEventSink for RecordingSink {
    fn emit(&self, event: &ServiceEvent) -> Result<(), ServiceEventError> {
        self.0.lock().unwrap().push(event.clone());
        Ok(())
    }
}

#[rstest]
#[case(PodLockCallType::Acquired, "_emit_acquired_lock_event", 1.0, 60.0)]
#[case(PodLockCallType::Released, "_emit_released_lock_event", 0.0, 17.0)]
#[tokio::test]
async fn delivers_pod_lock_service_events(
    #[case] call_type: PodLockCallType,
    #[case] name: &str,
    #[case] gauge: f64,
    #[case] duration: f64,
) {
    let now = DateTime::parse_from_rfc3339("2026-11-18T12:34:56+01:00").unwrap();
    let sink = Arc::new(RecordingSink::default());
    let pipeline = ServicePipeline::start(PipelineConfig {
        settings: WorkerSettings::default(),
        clock: Arc::new(FixedClock(now)),
        sink: Arc::<RecordingSink>::clone(&sink),
        registry: Arc::new(CallbackRegistry::new(8)),
    })
    .unwrap();
    pipeline.emit(PodLockEvent {
        call_type,
        gauge_labels: "spend:pod-1".to_owned(),
        gauge_value: gauge,
        duration_seconds: duration,
    });
    let report = pipeline.stop().await;
    assert_eq!(report.accepted, 1);
    assert_eq!(report.completed, 1);
    let recorded = sink.0.lock().unwrap();
    assert_eq!(recorded.len(), 1);
    assert_event(&recorded[0], name, gauge, duration, now);
}

fn assert_event(
    event: &ServiceEvent,
    name: &str,
    gauge: f64,
    duration: f64,
    now: DateTime<FixedOffset>,
) {
    assert_eq!(
        *event,
        ServiceEvent {
            call_id: String::new(),
            payload: ServiceLoggerPayload {
                is_error: false,
                error: None,
                service: ServiceType::PodLockManager,
                duration,
                call_type: name.to_owned(),
                caller: None,
                event_metadata: Some(ServiceEventMetadata {
                    gauge_labels: Some("spend:pod-1".to_owned()),
                    gauge_value: Some(gauge),
                }),
            },
            start_time: now,
            end_time: now,
        }
    );
}
