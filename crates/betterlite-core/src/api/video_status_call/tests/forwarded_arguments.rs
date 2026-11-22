use super::support::{
    Behavior, Bench, Environment, Event, Journal, Mode, Outcome, Production, SyncStub, VIDEO_ID,
    Wiring, call_with, handled, headers, kwargs, ready, request, sent, text, transforms,
    validations,
};
use crate::api::video_status_transport::records::{StatusMethod, SuppliedClient};

#[test]
fn forwarded_arguments_status_reach_the_handler() {
    let stub = SyncStub {
        journal: Journal::new(),
    };
    let mut bench = Bench::new(Outcome::answers());
    let mut request = request(VIDEO_ID);
    request.extra_headers = Some(headers(&[("x-extra", "one")]));
    request.extra_body = Some(kwargs(&[("extra", text("body"))]));
    request.extra_query = Some(kwargs(&[("poll", text("1"))]));
    ready(call_with(&mut bench, request, SuppliedClient::Sync(&stub)));
    let handling = handled(&bench.journal);
    assert_eq!(handling.extra_headers, Some(headers(&[("x-extra", "one")])));
    assert_eq!(
        handling.extra_body,
        Some(kwargs(&[("extra", text("body"))]))
    );
    assert_eq!(handling.client, Some("sync"));
}

#[test]
fn forwarded_arguments_status_transport_reaches_the_provider() {
    let journal = Journal::new();
    let wiring = Wiring::new(&journal);
    let environment = Environment::new();
    let prepost = environment.prepost();
    let mut production =
        Production::new(&journal, Behavior::Found, Mode::Answers, &wiring, &prepost);
    let mut request = request(VIDEO_ID);
    request.extra_headers = Some(headers(&[("x-extra", "one")]));
    request.extra_query = Some(kwargs(&[("poll", text("1"))]));
    request.extra_body = Some(kwargs(&[("extra", text("body"))]));
    let work = production
        .caller
        .video_status(request, None, &mut production.fixture.state);
    assert_eq!(ready(work).id, VIDEO_ID);
    assert_eq!(
        validations(&journal),
        vec![(headers(&[("x-extra", "one")]), false)]
    );
    assert_eq!(
        transforms(&journal),
        vec![(VIDEO_ID.to_owned(), headers(&[("x-provider", "gemini")]))]
    );
    let requests = sent(&journal);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, StatusMethod::Get);
    assert_eq!(requests[0].url, "http://mock.test/videos/video-1");
    let events = journal.events();
    assert!(events.contains(&Event::SyncClient));
    assert!(!events.contains(&Event::AsyncClient));
}
