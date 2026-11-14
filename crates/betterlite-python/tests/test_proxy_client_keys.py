import threading
import time
from collections.abc import Iterator, Mapping
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import TypedDict

import pytest
import requests
from litellm.proxy.client.credentials import CredentialsManagementClient as LiteCredentialsClient
from litellm.proxy.client.exceptions import UnauthorizedError as LiteUnauthorizedError
from litellm.proxy.client.keys import KeysManagementClient as LiteKeysClient
from proxy_client_support import HOST, RecordingServer, Seen
from pydantic import JsonValue

from betterlite.proxy.client._transport import HTTPError, Request, RequestError
from betterlite.proxy.client.credentials import CredentialsManagementClient
from betterlite.proxy.client.exceptions import UnauthorizedError
from betterlite.proxy.client.keys import (
    KeyGenerateOptions,
    KeyListOptions,
    KeysManagementClient,
    KeyUpdateError,
)

TIMEOUT = 5
OK = 200
DENIED = 401
BROKEN = 500
DENIED_BODY = "denied"
BOOM_BODY = "boom"
INFO_KEY = "sk-key?name=value"
REDACT_KEY = "sk-abcdefghijklmnopqrstuvwxyz123456"
CLOSED_URL = "http://127.0.0.1:1/unreachable"


class GetRequest(TypedDict):
    credential_name: str
    return_request: bool


REQUESTED_GET = GetRequest(credential_name="c1", return_request=True)


def compare_sent(
    server: RecordingServer,
    result: JsonValue | Request,
    expected: JsonValue,
) -> None:
    assert server.requests[-1] == server.requests[-2]
    assert result == expected


def as_request(result: JsonValue | Request) -> Request:
    match result:
        case Request():
            return result
        case dict() | list() | str() | bool() | int() | float() | None:
            raise AssertionError("the client sent the request instead of returning it")


def credential_call(
    client: LiteCredentialsClient | CredentialsManagementClient,
    operation: str,
    info: Mapping[str, JsonValue],
    values: Mapping[str, JsonValue],
) -> None:
    if operation == "create":
        client.create("c1", info, values)
    elif operation == "delete":
        client.delete("c1")
    elif operation == "get":
        client.get("c1")
    else:
        client.list()


def key_call(client: LiteKeysClient | KeysManagementClient, operation: str) -> None:
    models: list[str] = ["gpt-4"]
    keys: list[str] = ["sk-one"]
    match operation:
        case "generate":
            client.generate(models=models)
        case "delete":
            client.delete(keys=keys)
        case _:
            client.list(page=2)


@pytest.fixture(autouse=True)
def empty_home(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> Path:
    home = tmp_path / "home"
    home.mkdir()
    monkeypatch.setenv("HOME", str(home))
    return home


@pytest.fixture(autouse=False)
def server() -> Iterator[RecordingServer]:
    with RecordingServer() as recording:
        yield recording


@pytest.mark.parametrize(
    ("options", "query"),
    [
        (KeyListOptions(), ""),
        (KeyListOptions(page=0), "page=0"),
        (KeyListOptions(page=2), "page=2"),
        (KeyListOptions(size=10), "size=10"),
        (KeyListOptions(user_id="u1"), "user_id=u1"),
        (KeyListOptions(team_id="t1"), "team_id=t1"),
        (KeyListOptions(organization_id="o1"), "organization_id=o1"),
        (KeyListOptions(key_hash="h1"), "key_hash=h1"),
        (KeyListOptions(key_alias="a1"), "key_alias=a1"),
        (KeyListOptions(return_full_object=True), "return_full_object=true"),
        (KeyListOptions(include_team_keys=False), "include_team_keys=false"),
        (
            KeyListOptions(
                page=2,
                size=10,
                user_id="u1",
                team_id="t1",
                organization_id="o1",
                key_hash="h1",
                key_alias="a1",
                return_full_object=False,
                include_team_keys=True,
            ),
            (
                "page=2&size=10&user_id=u1&team_id=t1&organization_id=o1"
                "&key_hash=h1&key_alias=a1&return_full_object=false&include_team_keys=true"
            ),
        ),
    ],
)
def test_keys_request_list_option(
    server: RecordingServer, options: KeyListOptions, query: str
) -> None:
    lite = LiteKeysClient(server.url + "/", api_key="sk-test", timeout=TIMEOUT)
    ours = KeysManagementClient(server.url + "/", api_key="sk-test", timeout=TIMEOUT)
    server.reply(OK, "{}")
    lite.list(**options)
    server.reply(OK, "{}")
    compare_sent(server, ours.list(**options), {})
    assert server.requests[-1].query == query
    assert server.requests[-1].body is None


def test_keys_request_gen_option(server: RecordingServer) -> None:
    lite = LiteKeysClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    ours = KeysManagementClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    cases: list[tuple[KeyGenerateOptions, JsonValue]] = [
        (KeyGenerateOptions(models=["gpt-4", "gpt-4o"]), {"models": ["gpt-4", "gpt-4o"]}),
        (KeyGenerateOptions(models=[]), {"models": []}),
        (KeyGenerateOptions(aliases={"gpt-4": "gpt-4o"}), {"aliases": {"gpt-4": "gpt-4o"}}),
        (KeyGenerateOptions(aliases={}), {"aliases": {}}),
        (KeyGenerateOptions(spend=1.5), {"spend": 1.5}),
        (KeyGenerateOptions(spend=0.0), {"spend": 0.0}),
        (KeyGenerateOptions(duration="24h"), {"duration": "24h"}),
        (KeyGenerateOptions(duration=""), {"duration": ""}),
        (KeyGenerateOptions(key_alias="alias"), {"key_alias": "alias"}),
        (KeyGenerateOptions(team_id="t1"), {"team_id": "t1"}),
        (KeyGenerateOptions(user_id="u1"), {"user_id": "u1"}),
        (KeyGenerateOptions(budget_id="b1"), {"budget_id": "b1"}),
        (KeyGenerateOptions(config={"region": "eu"}), {"config": {"region": "eu"}}),
        (
            KeyGenerateOptions(
                models=["gpt-4"],
                aliases={"gpt-4": "gpt-4o"},
                spend=2.5,
                duration="7d",
                key_alias="alias",
                team_id="t1",
                user_id="u1",
                budget_id="b1",
                config={"region": "eu"},
            ),
            {
                "models": ["gpt-4"],
                "aliases": {"gpt-4": "gpt-4o"},
                "spend": 2.5,
                "duration": "7d",
                "key_alias": "alias",
                "team_id": "t1",
                "user_id": "u1",
                "budget_id": "b1",
                "config": {"region": "eu"},
            },
        ),
    ]
    for options, body in cases:
        server.reply(OK, "{}")
        lite.generate(**options)
        server.reply(OK, "{}")
        compare_sent(server, ours.generate(**options), {})
        assert server.requests[-1].body == body


def test_keys_request_calls(server: RecordingServer) -> None:
    lite = LiteKeysClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    ours = KeysManagementClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    server.reply(OK, '{"deleted": true}')
    lite.delete(keys=["sk-one"], key_aliases=["alias-one"])
    server.reply(OK, '{"deleted": true}')
    result = ours.delete(keys=["sk-one"], key_aliases=["alias-one"])
    compare_sent(server, result, {"deleted": True})
    assert server.requests[-1].body == {"keys": ["sk-one"], "key_aliases": ["alias-one"]}
    server.reply(OK, '{"deleted": true}')
    lite.delete()
    server.reply(OK, '{"deleted": true}')
    compare_sent(server, ours.delete(), {"deleted": True})
    assert server.requests[-1].body == {"keys": None, "key_aliases": None}
    update_body: JsonValue = {
        "key": "sk-updated-key",
        "key_alias": "alias",
        "user_id": "u1",
        "team_id": "t1",
        "models": ["gpt-4"],
        "spend": 3.0,
        "duration": "30d",
        "aliases": {"gpt-4": "gpt-4o"},
    }
    server.reply(OK, '{"key": "sk-updated-key"}')
    lite.update(
        "sk-updated-key",
        models=["gpt-4"],
        aliases={"gpt-4": "gpt-4o"},
        spend=3.0,
        duration="30d",
        key_alias="alias",
        team_id="t1",
        user_id="u1",
    )
    server.reply(OK, '{"key": "sk-updated-key"}')
    update_result = ours.update(
        "sk-updated-key",
        models=["gpt-4"],
        aliases={"gpt-4": "gpt-4o"},
        spend=3.0,
        duration="30d",
        key_alias="alias",
        team_id="t1",
        user_id="u1",
    )
    compare_sent(server, update_result, {"key": "sk-updated-key"})
    assert server.requests[-1].body == update_body
    server.reply(OK, "{}")
    lite.update("sk-updated-key")
    server.reply(OK, "{}")
    compare_sent(server, ours.update("sk-updated-key"), {})
    assert server.requests[-1].body == {"key": "sk-updated-key"}
    server.reply(OK, "{}")
    lite.update("sk-updated-key", spend=0.0, duration="")
    server.reply(OK, "{}")
    compare_sent(server, ours.update("sk-updated-key", spend=0.0, duration=""), {})
    zero_body: JsonValue = {"key": "sk-updated-key", "spend": 0.0, "duration": ""}
    assert server.requests[-1].body == zero_body
    server.reply(OK, '{"key": "sk-info"}')
    lite.info(INFO_KEY)
    server.reply(OK, '{"key": "sk-info"}')
    compare_sent(server, ours.info(INFO_KEY), {"key": "sk-info"})
    assert server.requests[-1].path == "/key/info"
    assert server.requests[-1].query == f"key={INFO_KEY}"


def test_keys_request_return(server: RecordingServer) -> None:
    lite = LiteKeysClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    ours = KeysManagementClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    lite.list(page=2, return_request=True)
    listed = as_request(ours.list(page=2, return_request=True))
    assert (listed.method, listed.url) == ("GET", f"{server.url}/key/list")
    assert listed.params == {"page": "2"}
    headers = {"Content-Type": "application/json", "Authorization": "Bearer sk-test"}
    assert listed.headers == headers
    lite.generate(spend=1.0, return_request=True)
    generated = as_request(ours.generate(spend=1.0, return_request=True))
    assert (generated.method, generated.url) == ("POST", f"{server.url}/key/generate")
    assert generated.json == {"spend": 1.0}
    lite.delete(keys=["sk-one"], return_request=True)
    deleted = as_request(ours.delete(keys=["sk-one"], return_request=True))
    assert (deleted.method, deleted.url) == ("POST", f"{server.url}/key/delete")
    assert deleted.json == {"keys": ["sk-one"], "key_aliases": None}
    lite.info(INFO_KEY, return_request=True)
    informed = as_request(ours.info(INFO_KEY, return_request=True))
    assert (informed.method, informed.url) == ("GET", f"{server.url}/key/info?key={INFO_KEY}")
    assert server.requests == []


def test_keys_request_no_apikey(server: RecordingServer) -> None:
    lite = LiteKeysClient(server.url, timeout=TIMEOUT)
    ours = KeysManagementClient(server.url, timeout=TIMEOUT)
    body: JsonValue = {"duration": "24h"}
    server.reply(OK, '{"key": "sk-generated"}')
    lite.generate(duration="24h")
    server.reply(OK, '{"key": "sk-generated"}')
    result = ours.generate(duration="24h")
    expected = Seen(
        method="POST",
        path="/key/generate",
        query="",
        authorization=None,
        content_type="application/json",
        body=body,
    )
    assert result == {"key": "sk-generated"}
    assert server.requests[-1] == expected
    assert server.requests[-1] == server.requests[0]


def test_keys_error_denied(server: RecordingServer) -> None:
    lite = LiteKeysClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    ours = KeysManagementClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    for operation in ["generate", "delete", "list"]:
        server.reply(DENIED, DENIED_BODY)
        with pytest.raises(LiteUnauthorizedError) as expected:
            key_call(lite, operation)
        server.reply(DENIED, DENIED_BODY)
        with pytest.raises(UnauthorizedError) as actual:
            key_call(ours, operation)
        assert str(actual.value) == str(expected.value)
        assert actual.value.__cause__ is not None
        assert expected.value.__cause__ is None


@pytest.mark.parametrize(
    ("status", "body", "lite_error", "ours_error"),
    [
        (DENIED, DENIED_BODY, LiteUnauthorizedError, UnauthorizedError),
        (BROKEN, BOOM_BODY, requests.exceptions.HTTPError, HTTPError),
    ],
)
def test_keys_error_info(
    server: RecordingServer,
    status: int,
    body: str,
    lite_error: type[Exception],
    ours_error: type[Exception],
) -> None:
    lite = LiteKeysClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    ours = KeysManagementClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    server.reply(status, body)
    with pytest.raises(lite_error) as expected:
        lite.info(REDACT_KEY)
    server.reply(status, body)
    with pytest.raises(ours_error) as actual:
        ours.info(REDACT_KEY)
    assert str(actual.value) == str(expected.value)
    assert "REDACTED" in str(actual.value)
    assert REDACT_KEY not in str(actual.value)
    assert actual.value.__cause__ is None


@pytest.mark.parametrize(
    ("target", "status", "body", "message"),
    [
        ("server", DENIED, DENIED_BODY, "Error updating key: denied"),
        ("server", BROKEN, BOOM_BODY, "Error updating key: boom"),
        ("server", OK, "not json", "Error updating key: not json"),
        ("closed", 0, "", "Error updating key: None"),
    ],
)
def test_keys_error_update(
    server: RecordingServer,
    target: str,
    status: int,
    body: str,
    message: str,
) -> None:
    url = server.url if target == "server" else CLOSED_URL
    lite = LiteKeysClient(url, api_key="sk-test", timeout=TIMEOUT)
    ours = KeysManagementClient(url, api_key="sk-test", timeout=TIMEOUT)
    if target == "server":
        server.reply(status, body)
    with pytest.raises(Exception, match=r"^Error updating key: ") as expected:
        lite.update("sk-updated-key", spend=1.0)
    if target == "server":
        server.reply(status, body)
    with pytest.raises(KeyUpdateError) as actual:
        ours.update("sk-updated-key", spend=1.0)
    assert str(actual.value) == message
    assert str(actual.value) == str(expected.value)
    assert actual.value.__cause__ is not None
    assert expected.value.__cause__ is None


class _StallHandler(BaseHTTPRequestHandler):
    def do_POST(self) -> None:
        self.send_response(OK)
        self.send_header("Content-Length", "100")
        self.end_headers()
        self.wfile.write(b"partial")
        time.sleep(1.5)


class StallingServer(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self) -> None:
        super().__init__((HOST, 0), _StallHandler)
        thread = threading.Thread(target=self.serve_forever, daemon=True)
        thread.start()

    def close(self) -> None:
        self.shutdown()
        self.server_close()


def test_keys_error_stalled() -> None:
    stalling = StallingServer()
    url = f"http://{HOST}:{stalling.server_port}"
    lite = LiteKeysClient(url, api_key="sk-test", timeout=1)
    ours = KeysManagementClient(url, api_key="sk-test", timeout=1)
    with pytest.raises(Exception, match=r"^Error updating key: None$") as expected:
        lite.update("sk-updated-key")
    with pytest.raises(KeyUpdateError) as actual:
        ours.update("sk-updated-key")
    stalling.close()
    assert str(actual.value) == "Error updating key: None"
    assert str(actual.value) == str(expected.value)
    assert actual.value.__cause__ is not None


def test_keys_error_not_object(server: RecordingServer) -> None:
    lite = LiteKeysClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    ours = KeysManagementClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    lite_credentials = LiteCredentialsClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    our_credentials = CredentialsManagementClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    server.reply(OK, "[]")
    lite.list()
    server.reply(OK, "[]")
    with pytest.raises(RequestError) as listed:
        ours.list()
    server.reply(OK, "[]")
    lite.info(INFO_KEY)
    server.reply(OK, "[]")
    with pytest.raises(RequestError) as informed:
        ours.info(INFO_KEY)
    server.reply(OK, "[]")
    lite_credentials.list()
    server.reply(OK, "[]")
    with pytest.raises(RequestError) as credential_list:
        our_credentials.list()
    server.reply(OK, "[]")
    lite.update("sk-updated-key")
    server.reply(OK, "[]")
    with pytest.raises(KeyUpdateError) as updated:
        ours.update("sk-updated-key")
    assert str(listed.value) == "the key reply is not a JSON object"
    assert str(informed.value) == "the key reply is not a JSON object"
    assert str(credential_list.value) == "the credential reply is not a JSON object"
    assert str(updated.value) == "Error updating key: []"


def test_keys_error_passthrough(server: RecordingServer) -> None:
    lite = LiteKeysClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    ours = KeysManagementClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    for operation in ["generate", "delete", "list"]:
        server.reply(BROKEN, BOOM_BODY)
        with pytest.raises(requests.exceptions.HTTPError) as expected:
            key_call(lite, operation)
        server.reply(BROKEN, BOOM_BODY)
        with pytest.raises(HTTPError) as actual:
            key_call(ours, operation)
        assert str(actual.value) == str(expected.value)
    lite_credentials = LiteCredentialsClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    our_credentials = CredentialsManagementClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    info: Mapping[str, JsonValue] = {"type": "openai"}
    values: Mapping[str, JsonValue] = {"api_key": "sk-credential-value-123456"}
    for operation in ["list", "create", "delete", "get"]:
        server.reply(BROKEN, BOOM_BODY)
        with pytest.raises(requests.exceptions.HTTPError) as expected:
            credential_call(lite_credentials, operation, info, values)
        server.reply(BROKEN, BOOM_BODY)
        with pytest.raises(HTTPError) as actual:
            credential_call(our_credentials, operation, info, values)
        assert str(actual.value) == str(expected.value)


def test_credentials_collection(server: RecordingServer) -> None:
    lite = LiteCredentialsClient(server.url + "/", api_key="sk-test", timeout=TIMEOUT)
    ours = CredentialsManagementClient(server.url + "/", api_key="sk-test", timeout=TIMEOUT)
    server.reply(OK, '{"credentials": []}')
    lite.list()
    server.reply(OK, '{"credentials": []}')
    compare_sent(server, ours.list(), {"credentials": []})
    assert server.requests[-1].method == "GET"
    assert server.requests[-1].path == "/credentials"
    assert server.requests[-1].body is None
    info: Mapping[str, JsonValue] = {"type": "openai"}
    values: Mapping[str, JsonValue] = {"api_key": "sk-credential-value-123456"}
    expected: JsonValue = {
        "credential_name": "c1",
        "credential_info": {"type": "openai"},
        "credential_values": {"api_key": "sk-credential-value-123456"},
    }
    server.reply(OK, '{"credential_name": "c1"}')
    lite.create("c1", info, values)
    server.reply(OK, '{"credential_name": "c1"}')
    compare_sent(server, ours.create("c1", info, values), {"credential_name": "c1"})
    assert server.requests[-1].method == "POST"
    assert server.requests[-1].path == "/credentials"
    assert server.requests[-1].body == expected


def test_credentials_named(server: RecordingServer) -> None:
    lite = LiteCredentialsClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    ours = CredentialsManagementClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    server.reply(OK, '{"deleted": true}')
    lite.delete("my-credential")
    server.reply(OK, '{"deleted": true}')
    compare_sent(server, ours.delete("my-credential"), {"deleted": True})
    assert (server.requests[-1].method, server.requests[-1].path) == (
        "DELETE",
        "/credentials/my-credential",
    )
    server.reply(OK, '{"credential_name": "c1"}')
    lite.get("my-credential")
    server.reply(OK, '{"credential_name": "c1"}')
    compare_sent(server, ours.get("my-credential"), {"credential_name": "c1"})
    assert (server.requests[-1].method, server.requests[-1].path) == (
        "GET",
        "/credentials/by_name/my-credential",
    )


def test_credentials_denied(server: RecordingServer) -> None:
    lite = LiteCredentialsClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    ours = CredentialsManagementClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    info: Mapping[str, JsonValue] = {"type": "openai"}
    values: Mapping[str, JsonValue] = {"api_key": "sk-credential-value-123456"}
    for operation in ["list", "create", "delete", "get"]:
        server.reply(DENIED, DENIED_BODY)
        with pytest.raises(LiteUnauthorizedError) as expected:
            credential_call(lite, operation, info, values)
        server.reply(DENIED, DENIED_BODY)
        with pytest.raises(UnauthorizedError) as actual:
            credential_call(ours, operation, info, values)
        assert str(actual.value) == str(expected.value)
        assert actual.value.__cause__ is not None
        assert expected.value.__cause__ is None
        assert server.requests[-1].authorization == "Bearer sk-test"


def test_credentials_return(server: RecordingServer) -> None:
    lite = LiteCredentialsClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    ours = CredentialsManagementClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    info: Mapping[str, JsonValue] = {"type": "openai"}
    values: Mapping[str, JsonValue] = {"api_key": "sk-credential-value-123456"}
    expected: JsonValue = {
        "credential_name": "c1",
        "credential_info": {"type": "openai"},
        "credential_values": {"api_key": "sk-credential-value-123456"},
    }
    lite.list(return_request=True)
    lite.create("c1", info, values, return_request=True)
    lite.delete("c1", return_request=True)
    lite.get(**REQUESTED_GET)
    prepared = [
        as_request(ours.list(return_request=True)),
        as_request(ours.create("c1", info, values, return_request=True)),
        as_request(ours.delete("c1", return_request=True)),
        as_request(ours.get(**REQUESTED_GET)),
    ]
    expected_targets = [
        ("GET", f"{server.url}/credentials"),
        ("POST", f"{server.url}/credentials"),
        ("DELETE", f"{server.url}/credentials/c1"),
        ("GET", f"{server.url}/credentials/by_name/c1"),
    ]
    assert [(item.method, item.url) for item in prepared] == expected_targets
    assert prepared[1].json == expected
    assert server.requests == []
