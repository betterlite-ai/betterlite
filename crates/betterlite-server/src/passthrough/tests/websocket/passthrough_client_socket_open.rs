use rstest::rstest;

use crate::passthrough::websocket::{WebSocket, WebSocketState, client_socket_is_open};

#[rstest]
#[case::both_connecting(WebSocketState::Connecting, WebSocketState::Connecting, true)]
#[case::client_connecting(WebSocketState::Connecting, WebSocketState::Connected, true)]
#[case::application_connecting(WebSocketState::Connected, WebSocketState::Connecting, true)]
#[case::both_connected(WebSocketState::Connected, WebSocketState::Connected, true)]
#[case::client_disconnected(WebSocketState::Disconnected, WebSocketState::Connected, false)]
#[case::application_disconnected(WebSocketState::Connected, WebSocketState::Disconnected, false)]
#[case::both_disconnected(WebSocketState::Disconnected, WebSocketState::Disconnected, false)]
#[case::client_disconnected_first(WebSocketState::Disconnected, WebSocketState::Connecting, false)]
#[case::application_disconnected_first(
    WebSocketState::Connecting,
    WebSocketState::Disconnected,
    false
)]
fn needs_both_halves_live(
    #[case] client_state: WebSocketState,
    #[case] application_state: WebSocketState,
    #[case] expected: bool,
) {
    let socket = WebSocket {
        client_state,
        application_state,
    };
    assert_eq!(client_socket_is_open(socket), expected);
}
