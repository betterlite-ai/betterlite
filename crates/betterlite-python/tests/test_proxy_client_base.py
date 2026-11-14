import json
from collections.abc import Callable, Iterator
from dataclasses import dataclass
from functools import partial
from pathlib import Path

import pytest
import requests
from litellm.proxy.client.exceptions import NotFoundError as LiteLLMNotFoundError
from litellm.proxy.client.exceptions import UnauthorizedError as LiteLLMUnauthorizedError
from litellm.proxy.client.health import HealthManagementClient as LiteLLMHealthClient
from litellm.proxy.client.model_groups import (
    ModelGroupsManagementClient as LiteLLMModelGroupsClient,
)
from litellm.proxy.client.users import UsersManagementClient as LiteLLMUsersClient
from proxy_client_support import RecordingServer
from pydantic import JsonValue

from betterlite.proxy.client import _bridge
from betterlite.proxy.client._transport import HTTPError, Request, RequestError, Response, send
from betterlite.proxy.client.exceptions import NotFoundError, UnauthorizedError
from betterlite.proxy.client.health import HealthManagementClient
from betterlite.proxy.client.http_client import HTTPClient
from betterlite.proxy.client.model_groups import ModelGroupsManagementClient
from betterlite.proxy.client.users import UsersManagementClient

TIMEOUT = 5
OK = 200
LEAKY_TEXT = "api_key=sk-abcdefghijklmnop"
CLOSED_URL = "http://127.0.0.1:1/unreachable"

LiteCall = Callable[[], JsonValue | requests.Request | requests.Response]


@dataclass(frozen=True)
class Reply:
    status: int
    body: str


@pytest.fixture(autouse=True)
def empty_home(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> Path:
    home = tmp_path / "home"
    home.mkdir()
    monkeypatch.setenv("HOME", str(home))
    return home


@pytest.fixture
def server() -> Iterator[RecordingServer]:
    with RecordingServer() as recording:
        yield recording


def get_checked(url: str) -> requests.Response:
    response = requests.request("GET", url, timeout=TIMEOUT)
    response.raise_for_status()
    return response


def compare_call(
    server: RecordingServer,
    reference: LiteCall,
    ours_call: Callable[[], JsonValue | Response],
    expected: JsonValue,
    reply: Reply,
) -> None:
    server.reply(reply.status, reply.body)
    reference()
    server.reply(reply.status, reply.body)
    result = ours_call()
    match result:
        case Response():
            assert result.status_code == reply.status
        case dict() | list() | str() | bool() | int() | float() | None:
            assert result == expected
    assert server.requests[1] == server.requests[0]


def compare_error(
    server: RecordingServer,
    lite_call: LiteCall,
    ours_call: Callable[[], JsonValue],
    errors: tuple[type[Exception], type[Exception]],
    reply: Reply,
) -> None:
    server.reply(reply.status, reply.body)
    with pytest.raises(errors[0]) as expected:
        lite_call()
    server.reply(reply.status, reply.body)
    with pytest.raises(errors[1]) as actual:
        ours_call()
    assert str(actual.value) == str(expected.value)


def test_transport_sends_query(server: RecordingServer) -> None:
    url = f"{server.url}/health/x"
    params = {"a": "1", "b": "2"}
    reference = partial(requests.request, "GET", url, params=params, timeout=TIMEOUT)
    request = Request(method="GET", url=url, params=params)
    compare_call(server, reference, partial(send, request, TIMEOUT), None, Reply(OK, "{}"))


def test_transport_sends_json(server: RecordingServer) -> None:
    url = f"{server.url}/x"
    reference = partial(requests.request, "POST", url, json={"a": 1}, timeout=TIMEOUT)
    request = Request(method="POST", url=url, json={"a": 1})
    compare_call(server, reference, partial(send, request, TIMEOUT), None, Reply(OK, "{}"))


def test_transport_caller_type(server: RecordingServer) -> None:
    url = f"{server.url}/x"
    headers = {"Content-Type": "application/json; charset=utf-8"}
    reference = partial(requests.request, "POST", url, json={"a": 1}, headers=dict(headers))
    request = Request(method="POST", url=url, json={"a": 1}, headers=dict(headers))
    compare_call(server, reference, partial(send, request, TIMEOUT), None, Reply(OK, "{}"))


def test_transport_raw_bytes(server: RecordingServer) -> None:
    url = f"{server.url}/raw"
    reference = partial(requests.request, "POST", url, data=b"raw-bytes", timeout=TIMEOUT)
    request = Request(method="POST", url=url, data=b"raw-bytes")
    compare_call(server, reference, partial(send, request, TIMEOUT), None, Reply(OK, "{}"))
    assert server.requests[0].content_type is None
    assert server.requests[0].body == "raw-bytes"


def test_transport_error_text(server: RecordingServer) -> None:
    for status in (404, 500):
        url = f"{server.url}/missing"
        server.reply(status, "boom")
        expected = requests.request("GET", url, timeout=TIMEOUT)
        with pytest.raises(requests.exceptions.HTTPError) as expected_error:
            expected.raise_for_status()
        server.reply(status, "boom")
        response = send(Request(method="GET", url=url), TIMEOUT)
        with pytest.raises(HTTPError) as actual_error:
            response.raise_for_status()
        assert str(actual_error.value) == str(expected_error.value)
        assert actual_error.value.response is response


def test_transport_connection() -> None:
    with pytest.raises(RequestError) as caught:
        send(Request(method="GET", url=CLOSED_URL), TIMEOUT)
    assert caught.value.__cause__ is not None


def test_transport_timeout_error(server: RecordingServer) -> None:
    server.reply(OK, "{}", delay=0.5)
    request = Request(method="GET", url=f"{server.url}/slow")
    with pytest.raises(RequestError) as caught:
        send(request, 0.05)
    assert caught.value.__cause__ is not None


def test_exceptions_redact_string() -> None:
    expected = str(LiteLLMUnauthorizedError(LEAKY_TEXT))
    actual = str(UnauthorizedError(LEAKY_TEXT))
    assert actual == expected
    assert "sk-abcdefghijklmnop" not in actual


def test_exceptions_notfound() -> None:
    expected = str(LiteLLMNotFoundError(LEAKY_TEXT))
    actual = str(NotFoundError(LEAKY_TEXT))
    assert actual == expected
    assert "sk-abcdefghijklmnop" not in actual


def test_exceptions_redact_http(server: RecordingServer) -> None:
    url = f"{server.url}/user/list"
    server.reply(401, "denied")
    expected = requests.request("GET", url, timeout=TIMEOUT)
    with pytest.raises(requests.exceptions.HTTPError) as raised:
        expected.raise_for_status()
    server.reply(401, "denied")
    response = send(Request(method="GET", url=url), TIMEOUT)
    with pytest.raises(HTTPError) as ours:
        response.raise_for_status()
    assert str(UnauthorizedError(ours.value)) == str(LiteLLMUnauthorizedError(raised.value))


def test_exceptions_keep_response(server: RecordingServer) -> None:
    server.reply(404, "missing")
    response = send(Request(method="GET", url=f"{server.url}/user/info"), TIMEOUT)
    with pytest.raises(HTTPError) as caught:
        response.raise_for_status()
    wrapped = NotFoundError(caught.value)
    match wrapped.orig_exception:
        case HTTPError():
            assert wrapped.orig_exception.response is response
        case str():
            pytest.fail("the HTTPError was replaced by a string")


def test_http_client_joins_url(server: RecordingServer) -> None:
    headers = {"Authorization": "Bearer sk-test"}
    reference = partial(requests.request, "GET", f"{server.url}/health/x", headers=headers)
    ours = HTTPClient(base_url=f"{server.url}/", api_key="sk-test", timeout=TIMEOUT)
    call = partial(ours.request, "GET", "health/x")
    compare_call(server, reference, call, {"ok": True}, Reply(OK, '{"ok": true}'))


def test_http_client_key_override(server: RecordingServer) -> None:
    headers = {"Authorization": "Bearer sk-test"}
    reference = partial(requests.request, "GET", f"{server.url}/x", headers=dict(headers))
    ours = HTTPClient(base_url=server.url, api_key="sk-test", timeout=TIMEOUT)
    call = partial(ours.request, "GET", "/x", headers={"Authorization": "Bearer caller"})
    compare_call(server, reference, call, {}, Reply(OK, "{}"))
    assert server.requests[0].authorization == "Bearer sk-test"


def test_http_client_no_key(server: RecordingServer) -> None:
    reference = partial(requests.request, "GET", f"{server.url}/x", timeout=TIMEOUT)
    ours = HTTPClient(base_url=server.url, timeout=TIMEOUT)
    compare_call(server, reference, partial(ours.request, "GET", "/x"), {}, Reply(OK, "{}"))
    assert server.requests[0].authorization is None


def test_http_client_json_body(server: RecordingServer) -> None:
    url = f"{server.url}/user/new"
    headers = {"Authorization": "Bearer sk-test"}
    reference = partial(requests.request, "POST", url, json={"user_id": "u1"}, headers=headers)
    ours = HTTPClient(base_url=server.url, api_key="sk-test", timeout=TIMEOUT)
    call = partial(ours.request, "POST", "/user/new", json={"user_id": "u1"})
    compare_call(server, reference, call, {"ok": True}, Reply(OK, '{"ok": true}'))


def test_http_client_caller_type(server: RecordingServer) -> None:
    headers = {"content-type": "application/json; charset=utf-8"}
    reference = partial(requests.request, "POST", f"{server.url}/x", json={"a": 1}, headers=headers)
    ours = HTTPClient(base_url=server.url, timeout=TIMEOUT)
    call = partial(ours.request, "POST", "/x", json={"a": 1}, headers=dict(headers))
    compare_call(server, reference, call, {}, Reply(OK, "{}"))


def test_http_client_form_body(server: RecordingServer) -> None:
    data = {"a": "1", "b": "2"}
    reference = partial(requests.request, "POST", f"{server.url}/x", data=dict(data))
    ours = HTTPClient(base_url=server.url, timeout=TIMEOUT)
    call = partial(ours.request, "POST", "/x", data=data)
    compare_call(server, reference, call, {}, Reply(OK, "{}"))


def test_http_client_pairs_body(server: RecordingServer) -> None:
    data = [("a", "1"), ("a", "2")]
    reference = partial(requests.request, "POST", f"{server.url}/x", data=list(data))
    ours = HTTPClient(base_url=server.url, timeout=TIMEOUT)
    call = partial(ours.request, "POST", "/x", data=data)
    compare_call(server, reference, call, {}, Reply(OK, "{}"))


def test_http_client_bytes_body(server: RecordingServer) -> None:
    reference = partial(requests.request, "POST", f"{server.url}/x", data=b"raw-bytes")
    ours = HTTPClient(base_url=server.url, timeout=TIMEOUT)
    call = partial(ours.request, "POST", "/x", data=b"raw-bytes")
    compare_call(server, reference, call, {}, Reply(OK, "{}"))


def test_http_client_returns_json(server: RecordingServer) -> None:
    server.reply(OK, '{"ok": true}')
    ours = HTTPClient(base_url=server.url, timeout=TIMEOUT)
    assert ours.request("GET", "/x") == {"ok": True}


def test_http_client_401_is_http(server: RecordingServer) -> None:
    ours = HTTPClient(base_url=server.url, timeout=TIMEOUT)
    errors = (requests.exceptions.HTTPError, HTTPError)
    reference = partial(get_checked, f"{server.url}/x")
    call = partial(ours.request, "GET", "/x")
    compare_error(server, reference, call, errors, Reply(401, "denied"))


def test_health_readiness(server: RecordingServer) -> None:
    reply = Reply(OK, '{"status": "healthy", "litellm_version": "1.105.1"}')
    lite = LiteLLMHealthClient(server.url, timeout=TIMEOUT)
    ours = HealthManagementClient(server.url, timeout=TIMEOUT)
    expected: JsonValue = {"status": "healthy", "litellm_version": "1.105.1"}
    compare_call(server, lite.get_readiness, ours.get_readiness, expected, reply)


def test_health_server_version(server: RecordingServer) -> None:
    reply = Reply(OK, '{"status": "healthy", "litellm_version": "1.105.1"}')
    lite = LiteLLMHealthClient(server.url, timeout=TIMEOUT)
    ours = HealthManagementClient(server.url, timeout=TIMEOUT)
    compare_call(server, lite.get_server_version, ours.get_server_version, "1.105.1", reply)


def test_health_version_absent(server: RecordingServer) -> None:
    reply = Reply(OK, '{"status": "healthy"}')
    lite = LiteLLMHealthClient(server.url, timeout=TIMEOUT)
    ours = HealthManagementClient(server.url, timeout=TIMEOUT)
    compare_call(server, lite.get_server_version, ours.get_server_version, None, reply)


def test_model_groups_info(server: RecordingServer) -> None:
    reply = Reply(OK, '{"data": [{"model_group": "gpt-4"}]}')
    lite = LiteLLMModelGroupsClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    ours = ModelGroupsManagementClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    compare_call(server, lite.info, ours.info, [{"model_group": "gpt-4"}], reply)


def test_model_groups_request(server: RecordingServer) -> None:
    ours = ModelGroupsManagementClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    returned = ours.info(return_request=True)
    assert returned.method == "GET"
    assert returned.url == f"{server.url}/model_group/info"
    assert returned.headers == {"Authorization": "Bearer sk-test"}
    assert server.requests == []


def test_model_groups_401(server: RecordingServer) -> None:
    lite = LiteLLMModelGroupsClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    ours = ModelGroupsManagementClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    errors = (LiteLLMUnauthorizedError, UnauthorizedError)
    compare_error(server, lite.info, ours.info, errors, Reply(401, "denied"))


def test_users_list(server: RecordingServer) -> None:
    reply = Reply(OK, '{"users": [{"user_id": "u1"}]}')
    lite = LiteLLMUsersClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    ours = UsersManagementClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    compare_call(server, lite.list_users, ours.list_users, [{"user_id": "u1"}], reply)


def test_users_list_whole(server: RecordingServer) -> None:
    reply = Reply(OK, '{"count": 1}')
    lite = LiteLLMUsersClient(server.url, timeout=TIMEOUT)
    ours = UsersManagementClient(server.url, timeout=TIMEOUT)
    compare_call(server, lite.list_users, ours.list_users, {"count": 1}, reply)


def test_users_list_params(server: RecordingServer) -> None:
    params: dict[str, str] = {"page": "2"}
    lite = LiteLLMUsersClient(server.url, timeout=TIMEOUT)
    ours = UsersManagementClient(server.url, timeout=TIMEOUT)
    lite_call = partial(lite.list_users, params=params)
    call = partial(ours.list_users, params=params)
    compare_call(server, lite_call, call, [], Reply(OK, '{"users": []}'))


def test_users_get_user(server: RecordingServer) -> None:
    reply = Reply(OK, '{"user_id": "u1"}')
    lite = LiteLLMUsersClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    ours = UsersManagementClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    call = partial(ours.get_user, user_id="u1")
    compare_call(server, partial(lite.get_user, user_id="u1"), call, {"user_id": "u1"}, reply)


def test_users_get_no_query(server: RecordingServer) -> None:
    reply = Reply(OK, '{"user_id": "u1"}')
    lite = LiteLLMUsersClient(server.url, timeout=TIMEOUT)
    ours = UsersManagementClient(server.url, timeout=TIMEOUT)
    compare_call(server, lite.get_user, ours.get_user, {"user_id": "u1"}, reply)
    assert server.requests[0].query == ""


def test_users_get_v2(server: RecordingServer) -> None:
    reply = Reply(OK, '{"user_id": "u1"}')
    lite = LiteLLMUsersClient(server.url, timeout=TIMEOUT)
    ours = UsersManagementClient(server.url, timeout=TIMEOUT)
    call = partial(ours.get_user_v2, user_id="u1")
    compare_call(server, partial(lite.get_user_v2, user_id="u1"), call, {"user_id": "u1"}, reply)
    assert server.requests[0].path == "/v2/user/info"


def test_users_get_404(server: RecordingServer) -> None:
    lite = LiteLLMUsersClient(server.url, timeout=TIMEOUT)
    ours = UsersManagementClient(server.url, timeout=TIMEOUT)
    errors = (LiteLLMNotFoundError, NotFoundError)
    compare_error(server, lite.get_user, ours.get_user, errors, Reply(404, "user not found"))


def test_users_create(server: RecordingServer) -> None:
    reply = Reply(OK, '{"user_id": "u1"}')
    lite = LiteLLMUsersClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    ours = UsersManagementClient(server.url, api_key="sk-test", timeout=TIMEOUT)
    call = partial(ours.create_user, {"user_id": "u1"})
    lite_call = partial(lite.create_user, {"user_id": "u1"})
    compare_call(server, lite_call, call, {"user_id": "u1"}, reply)


def test_users_delete(server: RecordingServer) -> None:
    reply = Reply(OK, '{"deleted": true}')
    lite = LiteLLMUsersClient(server.url, timeout=TIMEOUT)
    ours = UsersManagementClient(server.url, timeout=TIMEOUT)
    call = partial(ours.delete_user, ["u1", "u2"])
    lite_call = partial(lite.delete_user, ["u1", "u2"])
    compare_call(server, lite_call, call, {"deleted": True}, reply)


def test_users_create_401(server: RecordingServer) -> None:
    lite = LiteLLMUsersClient(server.url, timeout=TIMEOUT)
    ours = UsersManagementClient(server.url, timeout=TIMEOUT)
    errors = (LiteLLMUnauthorizedError, UnauthorizedError)
    call = partial(ours.create_user, {"user_id": "u1"})
    lite_call = partial(lite.create_user, {"user_id": "u1"})
    compare_error(server, lite_call, call, errors, Reply(401, "denied"))


def test_users_delete_401(server: RecordingServer) -> None:
    lite = LiteLLMUsersClient(server.url, timeout=TIMEOUT)
    ours = UsersManagementClient(server.url, timeout=TIMEOUT)
    errors = (LiteLLMUnauthorizedError, UnauthorizedError)
    call = partial(ours.delete_user, ["u1"])
    compare_error(server, partial(lite.delete_user, ["u1"]), call, errors, Reply(401, "denied"))


def test_bridge_redacts() -> None:
    expected = str(LiteLLMUnauthorizedError(LEAKY_TEXT))
    assert _bridge.redact_string(LEAKY_TEXT) == expected
    assert _bridge.redact_string("the server is ready") == "the server is ready"


def test_bridge_key_from_file(empty_home: Path) -> None:
    directory = empty_home / ".litellm"
    directory.mkdir()
    record = {"base_url": "http://proxy:4000", "key": "sk-stored"}
    (directory / "token.json").write_text(json.dumps(record), encoding="utf-8")
    assert _bridge.get_gateway_api_key("http://proxy:4000") == "sk-stored"


def test_bridge_key_no_token(empty_home: Path) -> None:
    assert not (empty_home / ".litellm").exists()
    assert _bridge.get_gateway_api_key("http://proxy:4000") is None


def test_bridge_key_other_url(empty_home: Path) -> None:
    directory = empty_home / ".litellm"
    directory.mkdir()
    record = {"base_url": "http://proxy:4000", "key": "sk-stored"}
    (directory / "token.json").write_text(json.dumps(record), encoding="utf-8")
    assert _bridge.get_gateway_api_key("http://elsewhere:4000") is None
