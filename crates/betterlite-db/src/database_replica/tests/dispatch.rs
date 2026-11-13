#![cfg(test)]

use std::sync::atomic::Ordering;

use rstest::rstest;

use crate::database_replica::availability::ReadTarget;
use crate::database_replica::pinned::{WriterPinnedClient, pin_writer, writer_wrapper};
use crate::database_replica::routing::{Route, RouteError};

use super::fixtures::fakes::{FakeAttribute, FakeError, Fixture, Side};

fn routed<'fixture>(fixture: &'fixture Fixture, name: &str) -> Route<'fixture, FakeAttribute> {
    fixture
        .routing
        .attribute(name)
        .expect("the model attribute routes")
}

#[test]
fn initial_availability_flags() {
    let fixture = Fixture::new();
    assert!(
        !fixture
            .routing
            .availability
            .reader_unavailable
            .load(Ordering::SeqCst)
    );
    assert!(
        !fixture
            .routing
            .availability
            .writer_unavailable
            .load(Ordering::SeqCst)
    );
    assert_eq!(fixture.routing.read_target(), ReadTarget::Reader);
}

#[rstest]
#[case::find_first("find_first")]
#[case::find_first_or_raise("find_first_or_raise")]
#[case::find_many("find_many")]
#[case::find_unique("find_unique")]
#[case::find_unique_or_raise("find_unique_or_raise")]
#[case::count("count")]
#[case::group_by("group_by")]
#[case::query_first("query_first")]
#[case::query_raw("query_raw")]
fn every_model_read_method(#[case] method: &str) {
    let fixture = Fixture::new();
    fixture.writer().add_model("VerificationToken");
    fixture.reader().add_model("VerificationToken");
    let route = routed(&fixture, "VerificationToken");
    assert_eq!(route.select(method).side, Side::Reader);
    assert_eq!(route.select("create").side, Side::Writer);
}

#[rstest]
#[case::query_first("query_first", Side::Reader)]
#[case::query_raw("query_raw", Side::Reader)]
#[case::execute_raw("execute_raw", Side::Writer)]
#[case::batch("batch_", Side::Writer)]
#[case::transaction("tx", Side::Writer)]
#[case::model_method("find_many", Side::Writer)]
fn only_two_top_level_read_methods(#[case] name: &str, #[case] expected: Side) {
    let fixture = Fixture::new();
    fixture.writer().add_direct(name);
    fixture.reader().add_direct(name);
    assert_eq!(routed(&fixture, name).select(name).side, expected);
}

#[test]
fn raw_writes_and_transactions_pinned() {
    let fixture = Fixture::new();
    fixture.writer().add_direct("execute_raw");
    fixture.writer().add_direct("batch_execute");
    fixture.writer().add_direct("tx");
    fixture.reader().add_direct("execute_raw");
    fixture.reader().add_direct("batch_execute");
    fixture.reader().add_direct("tx");
    assert_eq!(
        routed(&fixture, "execute_raw").select("execute_raw").side,
        Side::Writer
    );
    assert_eq!(
        routed(&fixture, "batch_execute")
            .select("batch_execute")
            .side,
        Side::Writer
    );
    assert_eq!(routed(&fixture, "tx").select("tx").side, Side::Writer);
}

#[test]
fn read_target_ignores_writer_flag() {
    let fixture = Fixture::new();
    fixture.writer().add_direct("query_raw");
    fixture.reader().add_direct("query_raw");
    fixture
        .routing
        .availability
        .writer_unavailable
        .store(true, Ordering::SeqCst);
    assert_eq!(fixture.routing.read_target(), ReadTarget::Reader);
    assert_eq!(
        routed(&fixture, "query_raw").select("query_raw").side,
        Side::Reader
    );
    fixture
        .routing
        .availability
        .reader_unavailable
        .store(true, Ordering::SeqCst);
    assert_eq!(fixture.routing.read_target(), ReadTarget::Writer);
    assert_eq!(
        routed(&fixture, "query_raw").select("query_raw").side,
        Side::Writer
    );
}

#[test]
fn retained_accessor_observes_reader_flip() {
    let fixture = Fixture::new();
    fixture.writer().add_model("VerificationToken");
    fixture.reader().add_model("VerificationToken");
    let route = routed(&fixture, "VerificationToken");
    assert_eq!(route.select("find_many").side, Side::Reader);
    fixture
        .routing
        .availability
        .reader_unavailable
        .store(true, Ordering::SeqCst);
    assert_eq!(route.select("find_many").side, Side::Writer);
    fixture
        .routing
        .availability
        .reader_unavailable
        .store(false, Ordering::SeqCst);
    assert_eq!(route.select("find_many").side, Side::Reader);
}

#[test]
fn already_bound_method_keeps_target() {
    let fixture = Fixture::new();
    fixture.writer().add_model("VerificationToken");
    fixture.reader().add_model("VerificationToken");
    let route = routed(&fixture, "VerificationToken");
    let bound = route.select("count");
    assert_eq!(bound.side, Side::Reader);
    fixture
        .routing
        .availability
        .reader_unavailable
        .store(true, Ordering::SeqCst);
    assert_eq!(bound.side, Side::Reader);
    assert_eq!(route.select("count").side, Side::Writer);
}

#[test]
fn missing_reader_accessor_uses_writer() {
    let fixture = Fixture::new();
    fixture.writer().add_model("VerificationToken");
    let route = routed(&fixture, "VerificationToken");
    assert_eq!(route.select("find_many").side, Side::Writer);
}

#[test]
fn non_accessor_stays_writer() {
    let fixture = Fixture::new();
    fixture.writer().add_direct("connection_status");
    fixture.reader().fail_lookup("connection_status");
    let route = routed(&fixture, "connection_status");
    assert_eq!(route.select("connection_status").side, Side::Writer);
    assert!(!fixture.has_event("reader:attribute:connection_status"));
}

#[test]
fn writer_lookup_error_propagates() {
    let fixture = Fixture::new();
    fixture.writer().fail_lookup("VerificationToken");
    fixture.reader().add_model("VerificationToken");
    assert!(matches!(
        fixture.routing.attribute("VerificationToken"),
        Err(RouteError::Lookup {
            source: FakeError::Lookup
        })
    ));
}

#[test]
fn absent_writer_attribute_is_named() {
    let fixture = Fixture::new();
    assert!(matches!(
        fixture.routing.attribute("missing_model"),
        Err(RouteError::MissingAttribute { .. })
    ));
}

#[test]
fn writer_pin_snapshot() {
    let fixture = Fixture::new();
    fixture.writer().add_model("VerificationToken");
    fixture.reader().add_model("VerificationToken");
    let pinned = pin_writer(WriterPinnedClient::Routed(&fixture.routing));
    assert!(matches!(pinned, WriterPinnedClient::Writer(_)));
    fixture
        .routing
        .availability
        .writer_unavailable
        .store(true, Ordering::SeqCst);
    assert!(matches!(pinned, WriterPinnedClient::Writer(_)));
    assert_eq!(
        pinned
            .attribute("VerificationToken")
            .expect("the pinned writer serves the model")
            .select("find_many")
            .side,
        Side::Writer
    );
    assert!(matches!(
        pin_writer(WriterPinnedClient::Routed(&fixture.routing)),
        WriterPinnedClient::Routed(_)
    ));
}

#[test]
fn degraded_writer_pin_allows_replica_read() {
    let fixture = Fixture::new();
    fixture.writer().add_model("VerificationToken");
    fixture.reader().add_model("VerificationToken");
    fixture
        .routing
        .availability
        .writer_unavailable
        .store(true, Ordering::SeqCst);
    let pinned = pin_writer(WriterPinnedClient::Routed(&fixture.routing));
    assert!(matches!(pinned, WriterPinnedClient::Routed(_)));
    assert_eq!(
        pinned
            .attribute("VerificationToken")
            .expect("the routed view serves the model")
            .select("find_many")
            .side,
        Side::Reader
    );
    fixture
        .routing
        .availability
        .writer_unavailable
        .store(false, Ordering::SeqCst);
    assert!(matches!(pinned, WriterPinnedClient::Routed(_)));
    assert!(matches!(
        pin_writer(WriterPinnedClient::Routed(&fixture.routing)),
        WriterPinnedClient::Writer(_)
    ));
}

#[test]
fn reader_lookup_error_propagates() {
    let fixture = Fixture::new();
    fixture.writer().add_model("VerificationToken");
    fixture.reader().fail_lookup("VerificationToken");
    assert!(matches!(
        fixture.routing.attribute("VerificationToken"),
        Err(RouteError::Lookup {
            source: FakeError::Lookup
        })
    ));
}

#[test]
fn pinned_writer_missing_attribute_is_named() {
    let fixture = Fixture::new();
    let pinned = pin_writer(WriterPinnedClient::Writer(&fixture.routing.writer));
    assert!(matches!(pinned, WriterPinnedClient::Writer(_)));
    assert!(matches!(
        pinned.attribute("missing_model"),
        Err(RouteError::MissingAttribute { .. })
    ));
}

#[test]
fn raw_writer_pin_never_falls_back() {
    let fixture = Fixture::new();
    assert_eq!(
        writer_wrapper(WriterPinnedClient::Writer(&fixture.routing.writer)).side(),
        Side::Writer
    );
    assert_eq!(
        writer_wrapper(WriterPinnedClient::Routed(&fixture.routing)).side(),
        Side::Writer
    );
    fixture
        .routing
        .availability
        .writer_unavailable
        .store(true, Ordering::SeqCst);
    assert_eq!(
        writer_wrapper(WriterPinnedClient::Routed(&fixture.routing)).side(),
        Side::Writer
    );
}

#[test]
fn mark_writer_only() {
    let fixture = Fixture::new();
    fixture
        .routing
        .availability
        .writer_unavailable
        .store(true, Ordering::SeqCst);
    fixture
        .routing
        .availability
        .reader_unavailable
        .store(true, Ordering::SeqCst);
    fixture
        .routing
        .availability
        .writer_unavailable
        .store(false, Ordering::SeqCst);
    assert!(
        !fixture
            .routing
            .availability
            .writer_unavailable
            .load(Ordering::SeqCst)
    );
    assert!(
        fixture
            .routing
            .availability
            .reader_unavailable
            .load(Ordering::SeqCst)
    );
    assert_eq!(fixture.routing.read_target(), ReadTarget::Writer);
    assert!(fixture.events().is_empty());
    assert!(fixture.captured.contents().is_empty());
}
