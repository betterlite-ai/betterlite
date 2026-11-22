use super::support::{
    Behavior, Environment, Event, Journal, Mode, Production, VIDEO_ID, Wiring, ready, request, sent,
};

#[test]
fn transport_status_sync_call_answers_through_the_standard_handler() {
    let journal = Journal::new();
    let wiring = Wiring::new(&journal);
    let environment = Environment::new();
    let prepost = environment.prepost();
    let mut production =
        Production::new(&journal, Behavior::Found, Mode::Answers, &wiring, &prepost);
    let work =
        production
            .caller
            .video_status(request(VIDEO_ID), None, &mut production.fixture.state);
    assert_eq!(ready(work).id, VIDEO_ID);
    let events = journal.events();
    assert!(events.contains(&Event::SyncClient));
    assert!(!events.contains(&Event::AsyncClient));
    let requests = sent(&journal);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, "http://mock.test/videos/video-1");
}

#[tokio::test]
async fn transport_status_async_call_answers_through_the_standard_handler() {
    let journal = Journal::new();
    let wiring = Wiring::new(&journal);
    let environment = Environment::new();
    let prepost = environment.prepost();
    let mut production =
        Production::new(&journal, Behavior::Found, Mode::Answers, &wiring, &prepost);
    let video = production
        .caller
        .avideo_status(request(VIDEO_ID), None, &mut production.fixture.state)
        .await
        .expect("the call answers");
    assert_eq!(video.id, VIDEO_ID);
    let events = journal.events();
    assert!(events.contains(&Event::AsyncClient));
    assert!(!events.contains(&Event::SyncClient));
    assert_eq!(sent(&journal).len(), 1);
}
