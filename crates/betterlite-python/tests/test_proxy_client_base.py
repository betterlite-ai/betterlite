import json
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

from betterlite.proxy.client import _bridge
from betterlite.proxy.client._transport import HTTPError, Request, RequestError, send
from betterlite.proxy.client.exceptions import NotFoundError, UnauthorizedError
from betterlite.proxy.client.health import HealthManagementClient
from betterlite.proxy.client.http_client import HTTPClient
from betterlite.proxy.client.model_groups import ModelGroupsManagementClient
from betterlite.proxy.client.users import UsersManagementClient

TIMEOUT = 5
OK = 200
LEAKY_TEXT = "api_key=sk-abcdefghijklmnop"
CLOSED_URL = "http://127.0.0.1:1/unreachable"


@pytest.fixture(autouse=True)
def empty_home(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> Path:
    home = tmp_path / "home"
    home.mkdir()
    monkeypatch.setenv("HOME", str(home))
    return home


def transport_sends_query() -> None:
    with RecordingServer() as server:
        server.reply(200, '{"ok": true}')
        expected = requests.request(
            "GET", f"{server.url}/health/x", params={"a": "1", "b": "2"}, timeout=TIMEOUT
        )
        server.reply(200, '{"ok": true}')
        request = Request(method="GET", url=f"{server.url}/health/x", params={"a": "1", "b": "2"})
        response = send(request, TIMEOUT)
        assert response.status_code == expected.status_code
        assert response.text == expected.text
        assert server.requests[1] == server.requests[0]


def transport_sends_json() -> None:
    with RecordingServer() as server:
        server.reply(200, "{}")
        payload: dict[str, int] = {"a": 1}
        expected = requests.post(f"{server.url}/x", json=payload, timeout=TIMEOUT)
        server.reply(200, "{}")
        response = send(Request(method="POST", url=f"{server.url}/x", json={"a": 1}), TIMEOUT)
        assert response.status_code == expected.status_code
        assert server.requests[1] == server.requests[0]


def transport_caller_type() -> None:
    headers = {"Content-Type": "application/json; charset=utf-8"}
    with RecordingServer() as server:
        server.reply(200, "{}")
        payload: dict[str, int] = {"a": 1}
        requests.post(f"{server.url}/x", json=payload, headers=dict(headers), timeout=TIMEOUT)
        server.reply(200, "{}")
        request = Request(
            method="POST", url=f"{server.url}/x", json={"a": 1}, headers=dict(headers)
        )
        send(request, TIMEOUT)
        assert server.requests[1] == server.requests[0]


def transport_raw_bytes() -> None:
    with RecordingServer() as server:
        server.reply(200, "{}")
        requests.post(f"{server.url}/raw", data=b"raw-bytes", timeout=TIMEOUT)
        server.reply(200, "{}")
        request = Request(method="POST", url=f"{server.url}/raw", data=b"raw-bytes")
        send(request, TIMEOUT)
        assert server.requests[1] == server.requests[0]
        assert server.requests[0].content_type is None
        assert server.requests[0].body == "raw-bytes"


def transport_client_error_text() -> None:
    with RecordingServer() as server:
        server.reply(404, '{"error": "missing"}')
        expected = requests.request("GET", f"{server.url}/missing", timeout=TIMEOUT)
        with pytest.raises(requests.exceptions.HTTPError) as expected_error:
            expected.raise_for_status()
        server.reply(404, '{"error": "missing"}')
        response = send(Request(method="GET", url=f"{server.url}/missing"), TIMEOUT)
        with pytest.raises(HTTPError) as actual_error:
            response.raise_for_status()
        assert str(actual_error.value) == str(expected_error.value)
        assert actual_error.value.response is response


def transport_server_error_text() -> None:
    with RecordingServer() as server:
        server.reply(500, "boom")
        expected = requests.request("GET", f"{server.url}/broken", timeout=TIMEOUT)
        with pytest.raises(requests.exceptions.HTTPError) as expected_error:
            expected.raise_for_status()
        server.reply(500, "boom")
        response = send(Request(method="GET", url=f"{server.url}/broken"), TIMEOUT)
        with pytest.raises(HTTPError) as actual_error:
            response.raise_for_status()
        assert str(actual_error.value) == str(expected_error.value)


def transport_connection_error() -> None:
    with pytest.raises(RequestError) as caught:
        send(Request(method="GET", url=CLOSED_URL), TIMEOUT)
    assert caught.value.__cause__ is not None


def transport_timeout_error() -> None:
    with RecordingServer() as server:
        server.reply(200, "{}", delay=0.5)
        request = Request(method="GET", url=f"{server.url}/slow")
        with pytest.raises(RequestError) as caught:
            send(request, 0.05)
        assert caught.value.__cause__ is not None


def exceptions_redact_string() -> None:
    expected = str(LiteLLMUnauthorizedError(LEAKY_TEXT))
    actual = str(UnauthorizedError(LEAKY_TEXT))
    assert actual == expected
    assert "sk-abcdefghijklmnop" not in actual


def exceptions_redact_http() -> None:
    with RecordingServer() as server:
        server.reply(401, "denied")
        expected_response = requests.request("GET", f"{server.url}/user/list", timeout=TIMEOUT)
        with pytest.raises(requests.exceptions.HTTPError) as expected_error:
            expected_response.raise_for_status()
        expected = str(LiteLLMUnauthorizedError(expected_error.value))
        server.reply(401, "denied")
        response = send(Request(method="GET", url=f"{server.url}/user/list"), TIMEOUT)
        with pytest.raises(HTTPError) as actual_error:
            response.raise_for_status()
        assert str(UnauthorizedError(actual_error.value)) == expected


def exceptions_keep_response() -> None:
    with RecordingServer() as server:
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


def exceptions_notfound_redacts() -> None:
    expected = str(LiteLLMNotFoundError(LEAKY_TEXT))
    actual = str(NotFoundError(LEAKY_TEXT))
    assert actual == expected
    assert "sk-abcdefghijklmnop" not in actual


def http_client_joins_url() -> None:
    with RecordingServer() as server:
        server.reply(200, '{"ok": true}')
        expected = requests.request("GET", f"{server.url}/health/x", timeout=TIMEOUT)
        server.reply(200, '{"ok": true}')
        ours = HTTPClient(base_url=f"{server.url}/", api_key="sk-test", timeout=TIMEOUT)
        assert ours.request("GET", "health/x") == {"ok": True}
        assert server.requests[1] == server.requests[0]
        assert expected.status_code == OK


def http_client_key_override() -> None:
    with RecordingServer() as server:
        server.reply(200, "{}")
        requests.request(
            "GET", f"{server.url}/x", headers={"Authorization": "Bearer sk-test"}, timeout=TIMEOUT
        )
        server.reply(200, "{}")
        ours = HTTPClient(base_url=server.url, api_key="sk-test", timeout=TIMEOUT)
        ours.request("GET", "/x", headers={"Authorization": "Bearer caller"})
        assert server.requests[1] == server.requests[0]
        assert server.requests[0].authorization == "Bearer sk-test"


def http_client_no_key() -> None:
    with RecordingServer() as server:
        server.reply(200, "{}")
        requests.request("GET", f"{server.url}/x", timeout=TIMEOUT)
        server.reply(200, "{}")
        HTTPClient(base_url=server.url, timeout=TIMEOUT).request("GET", "/x")
        assert server.requests[1] == server.requests[0]
        assert server.requests[0].authorization is None


def http_client_json_body() -> None:
    with RecordingServer() as server:
        server.reply(200, '{"ok": true}')
        payload: dict[str, str] = {"user_id": "u1"}
        requests.post(
            f"{server.url}/user/new",
            json=payload,
            headers={"Authorization": "Bearer sk-test"},
            timeout=TIMEOUT,
        )
        server.reply(200, '{"ok": true}')
        ours = HTTPClient(base_url=server.url, api_key="sk-test", timeout=TIMEOUT)
        assert ours.request("POST", "/user/new", json={"user_id": "u1"}) == {"ok": True}
        assert server.requests[1] == server.requests[0]


def http_client_caller_type() -> None:
    headers = {"content-type": "application/json; charset=utf-8"}
    with RecordingServer() as server:
        server.reply(200, "{}")
        payload: dict[str, int] = {"a": 1}
        requests.post(f"{server.url}/x", json=payload, headers=dict(headers), timeout=TIMEOUT)
        server.reply(200, "{}")
        ours = HTTPClient(base_url=server.url, timeout=TIMEOUT)
        ours.request("POST", "/x", json={"a": 1}, headers=dict(headers))
        assert server.requests[1] == server.requests[0]


def http_client_form_body() -> None:
    with RecordingServer() as server:
        server.reply(200, "{}")
        requests.post(f"{server.url}/x", data={"a": "1", "b": "2"}, timeout=TIMEOUT)
        server.reply(200, "{}")
        ours = HTTPClient(base_url=server.url, timeout=TIMEOUT)
        ours.request("POST", "/x", data={"a": "1", "b": "2"})
        assert server.requests[1] == server.requests[0]


def http_client_pairs_body() -> None:
    with RecordingServer() as server:
        server.reply(200, "{}")
        requests.post(f"{server.url}/x", data=[("a", "1"), ("a", "2")], timeout=TIMEOUT)
        server.reply(200, "{}")
        ours = HTTPClient(base_url=server.url, timeout=TIMEOUT)
        ours.request("POST", "/x", data=[("a", "1"), ("a", "2")])
        assert server.requests[1] == server.requests[0]


def http_client_bytes_body() -> None:
    with RecordingServer() as server:
        server.reply(200, "{}")
        requests.post(f"{server.url}/x", data=b"raw-bytes", timeout=TIMEOUT)
        server.reply(200, "{}")
        ours = HTTPClient(base_url=server.url, timeout=TIMEOUT)
        ours.request("POST", "/x", data=b"raw-bytes")
        assert server.requests[1] == server.requests[0]


def http_client_returns_json() -> None:
    with RecordingServer() as server:
        server.reply(200, '{"ok": true}')
        ours = HTTPClient(base_url=server.url, timeout=TIMEOUT)
        assert ours.request("GET", "/x") == {"ok": True}


def http_client_401_is_http() -> None:
    with RecordingServer() as server:
        server.reply(401, "denied")
        expected_response = requests.request("GET", f"{server.url}/x", timeout=TIMEOUT)
        with pytest.raises(requests.exceptions.HTTPError) as expected:
            expected_response.raise_for_status()
        server.reply(401, "denied")
        ours = HTTPClient(base_url=server.url, timeout=TIMEOUT)
        with pytest.raises(HTTPError) as actual:
            ours.request("GET", "/x")
        assert str(actual.value) == str(expected.value)


def health_readiness() -> None:
    body = '{"status": "healthy", "litellm_version": "1.105.1"}'
    with RecordingServer() as server:
        server.reply(200, body)
        lite = LiteLLMHealthClient(server.url, timeout=TIMEOUT)
        lite.get_readiness()
        server.reply(200, body)
        ours = HealthManagementClient(server.url, timeout=TIMEOUT)
        assert ours.get_readiness() == {"status": "healthy", "litellm_version": "1.105.1"}
        assert server.requests[1] == server.requests[0]


def health_server_version() -> None:
    body = '{"status": "healthy", "litellm_version": "1.105.1"}'
    with RecordingServer() as server:
        server.reply(200, body)
        lite = LiteLLMHealthClient(server.url, timeout=TIMEOUT)
        expected = lite.get_server_version()
        server.reply(200, body)
        ours = HealthManagementClient(server.url, timeout=TIMEOUT)
        assert ours.get_server_version() == expected
        assert expected == "1.105.1"


def health_version_absent() -> None:
    body = '{"status": "healthy"}'
    with RecordingServer() as server:
        server.reply(200, body)
        lite = LiteLLMHealthClient(server.url, timeout=TIMEOUT)
        expected = lite.get_server_version()
        server.reply(200, body)
        ours = HealthManagementClient(server.url, timeout=TIMEOUT)
        assert ours.get_server_version() == expected
        assert expected is None


def model_groups_info() -> None:
    body = '{"data": [{"model_group": "gpt-4"}]}'
    with RecordingServer() as server:
        server.reply(200, body)
        lite = LiteLLMModelGroupsClient(server.url, api_key="sk-test", timeout=TIMEOUT)
        lite.info()
        server.reply(200, body)
        ours = ModelGroupsManagementClient(server.url, api_key="sk-test", timeout=TIMEOUT)
        assert ours.info() == [{"model_group": "gpt-4"}]
        assert server.requests[1] == server.requests[0]


def model_groups_request() -> None:
    with RecordingServer() as server:
        ours = ModelGroupsManagementClient(server.url, api_key="sk-test", timeout=TIMEOUT)
        returned = ours.info(return_request=True)
        assert returned.method == "GET"
        assert returned.url == f"{server.url}/model_group/info"
        assert returned.headers == {"Authorization": "Bearer sk-test"}
        assert server.requests == []


def model_groups_401() -> None:
    with RecordingServer() as server:
        server.reply(401, "denied")
        lite = LiteLLMModelGroupsClient(server.url, api_key="sk-test", timeout=TIMEOUT)
        with pytest.raises(LiteLLMUnauthorizedError) as expected:
            lite.info()
        server.reply(401, "denied")
        ours = ModelGroupsManagementClient(server.url, api_key="sk-test", timeout=TIMEOUT)
        with pytest.raises(UnauthorizedError) as actual:
            ours.info()
        assert str(actual.value) == str(expected.value)


def users_list() -> None:
    body = '{"users": [{"user_id": "u1"}]}'
    with RecordingServer() as server:
        server.reply(200, body)
        lite = LiteLLMUsersClient(server.url, api_key="sk-test", timeout=TIMEOUT)
        lite.list_users()
        server.reply(200, body)
        ours = UsersManagementClient(server.url, api_key="sk-test", timeout=TIMEOUT)
        assert ours.list_users() == [{"user_id": "u1"}]
        assert server.requests[1] == server.requests[0]


def users_list_whole() -> None:
    body = '{"count": 1}'
    with RecordingServer() as server:
        server.reply(200, body)
        lite = LiteLLMUsersClient(server.url, timeout=TIMEOUT)
        lite.list_users()
        server.reply(200, body)
        ours = UsersManagementClient(server.url, timeout=TIMEOUT)
        assert ours.list_users() == {"count": 1}


def users_list_params() -> None:
    body = '{"users": []}'
    with RecordingServer() as server:
        server.reply(200, body)
        params: dict[str, str] = {"page": "2"}
        lite = LiteLLMUsersClient(server.url, timeout=TIMEOUT)
        lite.list_users(params=params)
        server.reply(200, body)
        ours = UsersManagementClient(server.url, timeout=TIMEOUT)
        ours.list_users(params=params)
        assert server.requests[1] == server.requests[0]


def users_get_user() -> None:
    body = '{"user_id": "u1"}'
    with RecordingServer() as server:
        server.reply(200, body)
        lite = LiteLLMUsersClient(server.url, api_key="sk-test", timeout=TIMEOUT)
        lite.get_user(user_id="u1")
        server.reply(200, body)
        ours = UsersManagementClient(server.url, api_key="sk-test", timeout=TIMEOUT)
        assert ours.get_user(user_id="u1") == {"user_id": "u1"}
        assert server.requests[1] == server.requests[0]


def users_get_no_query() -> None:
    body = '{"user_id": "u1"}'
    with RecordingServer() as server:
        server.reply(200, body)
        lite = LiteLLMUsersClient(server.url, timeout=TIMEOUT)
        lite.get_user()
        server.reply(200, body)
        ours = UsersManagementClient(server.url, timeout=TIMEOUT)
        ours.get_user()
        assert server.requests[1] == server.requests[0]
        assert server.requests[0].query == ""


def users_get_v2() -> None:
    body = '{"user_id": "u1"}'
    with RecordingServer() as server:
        server.reply(200, body)
        lite = LiteLLMUsersClient(server.url, timeout=TIMEOUT)
        lite.get_user_v2(user_id="u1")
        server.reply(200, body)
        ours = UsersManagementClient(server.url, timeout=TIMEOUT)
        ours.get_user_v2(user_id="u1")
        assert server.requests[1] == server.requests[0]
        assert server.requests[0].path == "/v2/user/info"


def users_get_404() -> None:
    with RecordingServer() as server:
        server.reply(404, "user not found")
        lite = LiteLLMUsersClient(server.url, timeout=TIMEOUT)
        with pytest.raises(LiteLLMNotFoundError) as expected:
            lite.get_user(user_id="u1")
        server.reply(404, "user not found")
        ours = UsersManagementClient(server.url, timeout=TIMEOUT)
        with pytest.raises(NotFoundError) as actual:
            ours.get_user(user_id="u1")
        assert str(actual.value) == str(expected.value)


def users_create() -> None:
    body = '{"user_id": "u1"}'
    with RecordingServer() as server:
        server.reply(200, body)
        lite = LiteLLMUsersClient(server.url, api_key="sk-test", timeout=TIMEOUT)
        lite.create_user({"user_id": "u1"})
        server.reply(200, body)
        ours = UsersManagementClient(server.url, api_key="sk-test", timeout=TIMEOUT)
        assert ours.create_user({"user_id": "u1"}) == {"user_id": "u1"}
        assert server.requests[1] == server.requests[0]


def users_delete() -> None:
    body = '{"deleted": true}'
    with RecordingServer() as server:
        server.reply(200, body)
        lite = LiteLLMUsersClient(server.url, timeout=TIMEOUT)
        lite.delete_user(["u1", "u2"])
        server.reply(200, body)
        ours = UsersManagementClient(server.url, timeout=TIMEOUT)
        assert ours.delete_user(["u1", "u2"]) == {"deleted": True}
        assert server.requests[1] == server.requests[0]


def users_create_401() -> None:
    with RecordingServer() as server:
        server.reply(401, "denied")
        lite = LiteLLMUsersClient(server.url, timeout=TIMEOUT)
        with pytest.raises(LiteLLMUnauthorizedError) as expected:
            lite.create_user({"user_id": "u1"})
        server.reply(401, "denied")
        ours = UsersManagementClient(server.url, timeout=TIMEOUT)
        with pytest.raises(UnauthorizedError) as actual:
            ours.create_user({"user_id": "u1"})
        assert str(actual.value) == str(expected.value)


def users_delete_401() -> None:
    with RecordingServer() as server:
        server.reply(401, "denied")
        lite = LiteLLMUsersClient(server.url, timeout=TIMEOUT)
        with pytest.raises(LiteLLMUnauthorizedError) as expected:
            lite.delete_user(["u1"])
        server.reply(401, "denied")
        ours = UsersManagementClient(server.url, timeout=TIMEOUT)
        with pytest.raises(UnauthorizedError) as actual:
            ours.delete_user(["u1"])
        assert str(actual.value) == str(expected.value)


def bridge_redacts() -> None:
    expected = str(LiteLLMUnauthorizedError(LEAKY_TEXT))
    assert _bridge.redact_string(LEAKY_TEXT) == expected
    assert _bridge.redact_string("the server is ready") == "the server is ready"


def bridge_key_from_file(empty_home: Path) -> None:
    directory = empty_home / ".litellm"
    directory.mkdir()
    record = {"base_url": "http://proxy:4000", "key": "sk-stored"}
    (directory / "token.json").write_text(json.dumps(record), encoding="utf-8")
    assert _bridge.get_gateway_api_key("http://proxy:4000") == "sk-stored"


def bridge_key_no_token(empty_home: Path) -> None:
    assert not (empty_home / ".litellm").exists()
    assert _bridge.get_gateway_api_key("http://proxy:4000") is None


def bridge_key_other_url(empty_home: Path) -> None:
    directory = empty_home / ".litellm"
    directory.mkdir()
    record = {"base_url": "http://proxy:4000", "key": "sk-stored"}
    (directory / "token.json").write_text(json.dumps(record), encoding="utf-8")
    assert _bridge.get_gateway_api_key("http://elsewhere:4000") is None
