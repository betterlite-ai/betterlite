import builtins
import json
from collections.abc import Iterator, Mapping, Sequence
from typing import Protocol, Unpack

import pytest
import requests
from litellm.proxy.client.chat import ChatClient as LiteLLMChatClient
from litellm.proxy.client.exceptions import NotFoundError as LiteLLMNotFoundError
from litellm.proxy.client.exceptions import UnauthorizedError as LiteLLMUnauthorizedError
from litellm.proxy.client.models import ModelsManagementClient as LiteLLMModelsClient
from litellm.proxy.client.teams import TeamsManagementClient as LiteLLMTeamsClient
from pydantic import JsonValue
from proxy_client_support import RecordingServer
from test_proxy_client_base import OK, TIMEOUT, Reply, compare_call, compare_error

from betterlite.proxy.client import chat as chat_module
from betterlite.proxy.client import models as models_module
from betterlite.proxy.client import teams as teams_module
from betterlite.proxy.client._transport import HTTPError, Request, RequestError, Response
from betterlite.proxy.client.chat import ChatClient, CompletionOptions
from betterlite.proxy.client.exceptions import NotFoundError, UnauthorizedError
from betterlite.proxy.client.models import ModelsManagementClient
from betterlite.proxy.client.teams import TeamListOptions, TeamsManagementClient

KEY = "sk-test"
MODEL = "gpt-4"
MESSAGES: list[dict[str, str]] = [{"role": "user", "content": "hi"}]
PARAMS: Mapping[str, JsonValue] = {"model": "openai/gpt-4"}
INFO: Mapping[str, JsonValue] = {"id": "m1"}
EMPTY: Mapping[str, JsonValue] = {}
MODEL_INFO_REPLY = (
    '{"data": [{"model_name": "other"}, '
    '{"model_name": "gpt-4", "model_info": {"id": "m1"}}]}'
)
STREAM_BODY = (
    ': keep-alive\n\n'
    "event: message\n\n"
    'data: {"id": "c1"}\n\n'
    "data: not-json\n\n"
    'data:  {"id": "c2"}\n\n'
    "data: [DONE] \n\n"
    'data: {"id": "c3"}\n\n'
)


class LiteModels(Protocol):
    def list(
        self, *, return_request: bool = False
    ) -> Sequence[Mapping[str, JsonValue]] | requests.Request: ...

    def info(
        self, *, return_request: bool = False
    ) -> Sequence[Mapping[str, JsonValue]] | requests.Request: ...

    def new(
        self, model_name: str, model_params: Mapping[str, JsonValue], *,
        model_info: Mapping[str, JsonValue] | None = None, return_request: bool = False,
    ) -> Mapping[str, JsonValue] | requests.Request: ...

    def update(
        self, model_id: str, model_params: Mapping[str, JsonValue], *,
        model_info: Mapping[str, JsonValue] | None = None, return_request: bool = False,
    ) -> Mapping[str, JsonValue] | requests.Request: ...

    def delete(
        self, model_id: str, *, return_request: bool = False
    ) -> Mapping[str, JsonValue] | requests.Request: ...

    def get(
        self, model_id: str | None = None, model_name: str | None = None, *,
        return_request: bool = False,
    ) -> Mapping[str, JsonValue] | requests.Request: ...


class LiteChat(Protocol):
    def completions(
        self, model: str, messages: list[dict[str, str]], *,
        return_request: bool = False, **options: Unpack[CompletionOptions],
    ) -> Mapping[str, JsonValue] | requests.Request: ...

    def completions_stream(
        self, model: str, messages: list[dict[str, str]],
        **options: Unpack[CompletionOptions],
    ) -> Iterator[Mapping[str, JsonValue]]: ...


class LiteTeams(Protocol):
    def list(
        self, user_id: str | None = None, organization_id: str | None = None
    ) -> Sequence[Mapping[str, JsonValue]]: ...

    def list_v2(self, **options: Unpack[TeamListOptions]) -> Mapping[str, JsonValue]: ...

    def get_available(self) -> Sequence[Mapping[str, JsonValue]]: ...


class TimeoutProbe:
    def __init__(self) -> None:
        self.calls: list[tuple[str, float]] = []

    def __call__(self, request: Request, timeout: float) -> Response:
        self.calls.append((request.url, timeout))
        return Response(status_code=OK, text='{"data": []}')


@pytest.fixture(autouse=False)
def server() -> Iterator[RecordingServer]:
    with RecordingServer() as recording:
        yield recording


@pytest.fixture(autouse=False)
def probe() -> Iterator[TimeoutProbe]:
    recording = TimeoutProbe()
    saved = (models_module.send, teams_module.send, chat_module.send, chat_module.stream)
    models_module.send = recording
    teams_module.send = recording
    chat_module.send = recording
    chat_module.stream = recording
    yield recording
    models_module.send, teams_module.send, chat_module.send, chat_module.stream = saved


def models_clients(server: RecordingServer) -> tuple[LiteModels, ModelsManagementClient]:
    lite: LiteModels = LiteLLMModelsClient(server.url, api_key=KEY, timeout=TIMEOUT)
    ours = ModelsManagementClient(server.url, api_key=KEY, timeout=TIMEOUT)
    return lite, ours


def chat_clients(server: RecordingServer) -> tuple[LiteChat, ChatClient]:
    lite: LiteChat = LiteLLMChatClient(server.url, api_key=KEY, timeout=TIMEOUT)
    ours = ChatClient(server.url, api_key=KEY, timeout=TIMEOUT)
    return lite, ours


def teams_clients(server: RecordingServer) -> tuple[LiteTeams, TeamsManagementClient]:
    lite: LiteTeams = LiteLLMTeamsClient(server.url, api_key=KEY, timeout=TIMEOUT)
    ours = TeamsManagementClient(server.url, api_key=KEY, timeout=TIMEOUT)
    return lite, ours


def test_client_timeouts(probe: TimeoutProbe) -> None:
    models_client = ModelsManagementClient("http://proxy:4000", api_key=KEY)
    teams_client = TeamsManagementClient("http://proxy:4000")
    chat_client = ChatClient("http://proxy:4000")
    assert models_client.list() == []
    assert teams_client.get_available() == {"data": []}
    assert chat_client.completions(MODEL, MESSAGES) == {"data": []}
    assert builtins.list(chat_client.completions_stream(MODEL, MESSAGES)) == []
    assert probe.calls == [
        ("http://proxy:4000/models", 30),
        ("http://proxy:4000/team/available", 30),
        ("http://proxy:4000/chat/completions", 600),
        ("http://proxy:4000/chat/completions", 600),
    ]
    custom = "http://other:4000"
    assert ModelsManagementClient(custom, timeout=7).info() == []
    assert TeamsManagementClient(custom, timeout=7).list() == {"data": []}
    assert ChatClient(custom, timeout=7).completions(MODEL, MESSAGES) == {"data": []}
    assert builtins.list(ChatClient(custom, timeout=7).completions_stream(MODEL, MESSAGES)) == []
    assert probe.calls[4:] == [
        (f"{custom}/v1/model/info", 7),
        (f"{custom}/team/list", 7),
        (f"{custom}/chat/completions", 7),
        (f"{custom}/chat/completions", 7),
    ]


def test_models_list(server: RecordingServer) -> None:
    lite, ours = models_clients(server)
    reply = Reply(OK, '{"data": [{"model_name": "gpt-4"}]}')
    compare_call(server, lite.list, ours.list, [{"model_name": "gpt-4"}], reply)


def test_models_info(server: RecordingServer) -> None:
    lite, ours = models_clients(server)
    reply = Reply(OK, '{"data": [{"model_name": "gpt-4"}]}')
    compare_call(server, lite.info, ours.info, [{"model_name": "gpt-4"}], reply)


def test_models_list_anonymous(server: RecordingServer) -> None:
    lite: LiteModels = LiteLLMModelsClient(server.url, timeout=TIMEOUT)
    ours = ModelsManagementClient(server.url, timeout=TIMEOUT)
    reply = Reply(OK, '{"data": []}')
    compare_call(server, lite.list, ours.list, [], reply)
    assert server.requests[0].authorization is None


def test_models_new(server: RecordingServer) -> None:
    lite, ours = models_clients(server)
    server.reply(OK, '{"model_id": "m1"}')
    expected = lite.new("gpt-4", PARAMS)
    server.reply(OK, '{"model_id": "m1"}')
    assert ours.new("gpt-4", PARAMS) == expected
    assert server.requests[1] == server.requests[0]


def test_models_new_info(server: RecordingServer) -> None:
    lite, ours = models_clients(server)
    server.reply(OK, '{"model_id": "m1"}')
    expected = lite.new("gpt-4", PARAMS, model_info=INFO)
    server.reply(OK, '{"model_id": "m1"}')
    assert ours.new("gpt-4", PARAMS, model_info=INFO) == expected
    assert server.requests[1] == server.requests[0]


def test_models_new_empty_info(server: RecordingServer) -> None:
    lite, ours = models_clients(server)
    server.reply(OK, '{"model_id": "m1"}')
    expected = lite.new("gpt-4", PARAMS, model_info=EMPTY)
    server.reply(OK, '{"model_id": "m1"}')
    assert ours.new("gpt-4", PARAMS, model_info=EMPTY) == expected
    assert server.requests[1] == server.requests[0]


def test_models_update(server: RecordingServer) -> None:
    lite, ours = models_clients(server)
    server.reply(OK, '{"model": "m1"}')
    expected = lite.update("m1", PARAMS)
    server.reply(OK, '{"model": "m1"}')
    assert ours.update("m1", PARAMS) == expected
    assert server.requests[1] == server.requests[0]


def test_models_update_info(server: RecordingServer) -> None:
    lite, ours = models_clients(server)
    server.reply(OK, '{"model": "m1"}')
    expected = lite.update("m1", PARAMS, model_info=INFO)
    server.reply(OK, '{"model": "m1"}')
    assert ours.update("m1", PARAMS, model_info=INFO) == expected
    assert server.requests[1] == server.requests[0]


def test_models_delete(server: RecordingServer) -> None:
    lite, ours = models_clients(server)
    server.reply(OK, '{"deleted": true}')
    expected = lite.delete("m1")
    server.reply(OK, '{"deleted": true}')
    assert ours.delete("m1") == expected
    assert server.requests[1] == server.requests[0]


def test_models_get_id(server: RecordingServer) -> None:
    lite, ours = models_clients(server)
    server.reply(OK, MODEL_INFO_REPLY)
    expected = lite.get(model_id="m1")
    server.reply(OK, MODEL_INFO_REPLY)
    assert ours.get(model_id="m1") == expected
    assert server.requests[1] == server.requests[0]


def test_models_get_name(server: RecordingServer) -> None:
    lite, ours = models_clients(server)
    server.reply(OK, MODEL_INFO_REPLY)
    expected = lite.get(model_name="gpt-4")
    server.reply(OK, MODEL_INFO_REPLY)
    assert ours.get(model_name="gpt-4") == expected
    assert server.requests[1] == server.requests[0]


def test_models_get_selector(server: RecordingServer) -> None:
    _, ours = models_clients(server)
    message = "Exactly one of model_id or model_name must be provided"
    with pytest.raises(ValueError, match=message):
        ours.get()
    with pytest.raises(ValueError, match=message):
        ours.get(model_id="m1", model_name="gpt-4")  # ast-grep-ignore: no-fallbacks
    assert server.requests == []


def test_models_list_missing(server: RecordingServer) -> None:
    lite, ours = models_clients(server)
    server.reply(404, "not found")
    with pytest.raises(requests.exceptions.HTTPError):
        lite.list()
    server.reply(404, "not found")
    with pytest.raises(HTTPError):
        ours.list()


def test_models_list_bad_reply(server: RecordingServer) -> None:
    _, ours = models_clients(server)
    for body in ('{"data": 3}', "[]"):
        server.reply(OK, body)
        with pytest.raises(RequestError) as caught:
            ours.list()
        server.reply(OK, body)
        with pytest.raises(RequestError):
            ours.info()
        assert str(caught.value) == "the models reply has no data list"


def test_models_unauthorized(server: RecordingServer) -> None:
    lite, ours = models_clients(server)
    errors = (LiteLLMUnauthorizedError, UnauthorizedError)
    reply = Reply(401, "denied")
    compare_error(server, lite.list, ours.list, errors, reply)
    compare_error(server, lite.info, ours.info, errors, reply)
    server.reply(401, "denied")
    with pytest.raises(LiteLLMUnauthorizedError) as expected:
        lite.new("gpt-4", PARAMS)
    server.reply(401, "denied")
    with pytest.raises(UnauthorizedError) as actual:
        ours.new("gpt-4", PARAMS)
    assert str(actual.value) == str(expected.value)
    server.reply(401, "Model not found")
    with pytest.raises(LiteLLMUnauthorizedError) as expected:
        lite.update("m1", PARAMS)
    server.reply(401, "Model not found")
    with pytest.raises(UnauthorizedError) as actual:
        ours.update("m1", PARAMS)
    assert str(actual.value) == str(expected.value)
    server.reply(401, "Model not found")
    with pytest.raises(LiteLLMUnauthorizedError) as expected:
        lite.delete("m1")
    server.reply(401, "Model not found")
    with pytest.raises(UnauthorizedError) as actual:
        ours.delete("m1")
    assert str(actual.value) == str(expected.value)


def test_models_request_only(server: RecordingServer) -> None:
    _, ours = models_clients(server)
    listed = ours.list(return_request=True)
    assert listed.url == f"{server.url}/models"
    assert listed.headers == {"Authorization": f"Bearer {KEY}"}
    detail = ours.info(return_request=True)
    assert detail.url == f"{server.url}/v1/model/info"
    created = ours.new("gpt-4", PARAMS, return_request=True)
    informed = ours.new("gpt-4", PARAMS, model_info=INFO, return_request=True)
    changed = ours.update("m1", PARAMS, return_request=True)
    removed = ours.delete("m1", return_request=True)
    bodies = [created.json, informed.json, changed.json, removed.json]
    assert json.dumps(bodies) == (
        '[{"model_name": "gpt-4", "litellm_params": {"model": "openai/gpt-4"}}, '
        '{"model_name": "gpt-4", "litellm_params": {"model": "openai/gpt-4"}, '
        '"model_info": {"id": "m1"}}, '
        '{"id": "m1", "litellm_params": {"model": "openai/gpt-4"}}, {"id": "m1"}]'
    )
    found = ours.get(model_id="m1", return_request=True)  # ast-grep-ignore: no-fallbacks
    assert found.url == f"{server.url}/v1/model/info"
    slashed = ModelsManagementClient(f"{server.url}/", api_key=KEY, timeout=TIMEOUT)
    assert slashed.list(return_request=True).url == f"{server.url}/models"
    assert server.requests == []


def test_models_missing(server: RecordingServer) -> None:
    lite, ours = models_clients(server)
    server.reply(404, "no such model")
    with pytest.raises(LiteLLMNotFoundError) as expected:
        lite.update("m9", PARAMS)
    server.reply(404, "no such model")
    with pytest.raises(NotFoundError) as actual:
        ours.update("m9", PARAMS)
    assert str(actual.value) == str(expected.value)
    assert server.requests[1] == server.requests[0]
    server.reply(500, "Model NOT FOUND in the database")
    with pytest.raises(LiteLLMNotFoundError) as expected:
        lite.update("m9", PARAMS)
    server.reply(500, "Model NOT FOUND in the database")
    with pytest.raises(NotFoundError) as actual:
        ours.update("m9", PARAMS)
    assert str(actual.value) == str(expected.value)
    server.reply(404, "no such model")
    with pytest.raises(LiteLLMNotFoundError) as expected:
        lite.delete("m9")
    server.reply(404, "no such model")
    with pytest.raises(NotFoundError) as actual:
        ours.delete("m9")
    assert str(actual.value) == str(expected.value)
    server.reply(500, "Model NOT FOUND in the database")
    with pytest.raises(LiteLLMNotFoundError) as expected:
        lite.delete("m9")
    server.reply(500, "Model NOT FOUND in the database")
    with pytest.raises(NotFoundError) as actual:
        ours.delete("m9")
    assert str(actual.value) == str(expected.value)
    server.reply(500, "boom")
    with pytest.raises(requests.exceptions.HTTPError) as expected:
        lite.update("m9", PARAMS)
    server.reply(500, "boom")
    with pytest.raises(HTTPError) as actual:
        ours.update("m9", PARAMS)
    assert str(actual.value) == str(expected.value)
    server.reply(404, "no such model")
    with pytest.raises(requests.exceptions.HTTPError):
        lite.new("gpt-4", PARAMS)
    server.reply(404, "no such model")
    with pytest.raises(HTTPError):
        ours.new("gpt-4", PARAMS)


def test_models_get_missing(server: RecordingServer) -> None:
    lite, ours = models_clients(server)
    server.reply(OK, MODEL_INFO_REPLY)
    with pytest.raises(LiteLLMNotFoundError) as expected:
        lite.get(model_id="m9")
    server.reply(OK, MODEL_INFO_REPLY)
    with pytest.raises(NotFoundError) as actual:
        ours.get(model_id="m9")
    assert str(actual.value) == str(expected.value)
    assert str(actual.value) == "Model with id=m9 not found"
    assert server.requests[1] == server.requests[0]
    match actual.value.orig_exception:
        case HTTPError() as http_error:
            assert http_error.response is not None
            assert http_error.response.status_code is None
        case str():
            pytest.fail("the not-found error carried a string")
    server.reply(OK, MODEL_INFO_REPLY)
    with pytest.raises(LiteLLMNotFoundError) as expected:
        lite.get(model_name="missing")
    server.reply(OK, MODEL_INFO_REPLY)
    with pytest.raises(NotFoundError) as actual:
        ours.get(model_name="missing")
    assert str(actual.value) == str(expected.value)
    assert str(actual.value) == "Model with model_name=missing not found"


def test_models_get_empty_selector(server: RecordingServer) -> None:
    lite, ours = models_clients(server)
    reply = '{"data": [{"model_name": "", "model_info": {"id": ""}}]}'
    server.reply(OK, reply)
    with pytest.raises(LiteLLMNotFoundError) as expected:
        lite.get(model_id="")
    server.reply(OK, reply)
    with pytest.raises(NotFoundError) as actual:
        ours.get(model_id="")
    assert str(actual.value) == str(expected.value)
    assert str(actual.value) == "Unknown error trying to find model"
    server.reply(OK, reply)
    with pytest.raises(LiteLLMNotFoundError) as expected:
        lite.get(model_name="")
    server.reply(OK, reply)
    with pytest.raises(NotFoundError) as actual:
        ours.get(model_name="")
    assert str(actual.value) == str(expected.value)
    assert server.requests[1] == server.requests[0]
    assert server.requests[3] == server.requests[2]


def test_models_get_bad_entry(server: RecordingServer) -> None:
    _, ours = models_clients(server)
    server.reply(OK, '{"data": ["no", {"model_name": "gpt-4"}, 3]}')
    assert ours.get(model_name="gpt-4") == {"model_name": "gpt-4"}


def test_chat_completions(server: RecordingServer) -> None:
    lite, ours = chat_clients(server)
    server.reply(OK, '{"id": "c1"}')
    expected = lite.completions(MODEL, MESSAGES)
    server.reply(OK, '{"id": "c1"}')
    assert ours.completions(MODEL, MESSAGES) == expected
    assert server.requests[1] == server.requests[0]


def test_chat_completions_no_key(server: RecordingServer) -> None:
    lite: LiteChat = LiteLLMChatClient(server.url, timeout=TIMEOUT)
    ours = ChatClient(server.url, timeout=TIMEOUT)
    server.reply(OK, '{"id": "c1"}')
    expected = lite.completions(MODEL, MESSAGES)
    server.reply(OK, '{"id": "c1"}')
    assert ours.completions(MODEL, MESSAGES) == expected
    server.reply(OK, STREAM_BODY)
    builtins.list(lite.completions_stream(MODEL, MESSAGES))
    server.reply(OK, STREAM_BODY)
    builtins.list(ours.completions_stream(MODEL, MESSAGES))
    assert server.requests[1] == server.requests[0]
    assert server.requests[3] == server.requests[2]
    assert server.requests[2].authorization is None
    assert server.requests[2].content_type == "application/json"


def test_chat_completions_options(server: RecordingServer) -> None:
    lite, ours = chat_clients(server)
    server.reply(OK, '{"id": "c1"}')
    expected = lite.completions(MODEL, MESSAGES, temperature=0.2, top_p=0.9, n=2)
    server.reply(OK, '{"id": "c1"}')
    assert ours.completions(MODEL, MESSAGES, temperature=0.2, top_p=0.9, n=2) == expected
    assert server.requests[1] == server.requests[0]


def test_chat_penalty_options(server: RecordingServer) -> None:
    lite, ours = chat_clients(server)
    server.reply(OK, '{"id": "c1"}')
    expected = lite.completions(MODEL, MESSAGES, max_tokens=32, presence_penalty=0.1)
    server.reply(OK, '{"id": "c1"}')
    assert ours.completions(MODEL, MESSAGES, max_tokens=32, presence_penalty=0.1) == expected
    assert server.requests[1] == server.requests[0]


def test_chat_completions_user(server: RecordingServer) -> None:
    lite, ours = chat_clients(server)
    server.reply(OK, '{"id": "c1"}')
    expected = lite.completions(MODEL, MESSAGES, frequency_penalty=0.3, user="u1")
    server.reply(OK, '{"id": "c1"}')
    assert ours.completions(MODEL, MESSAGES, frequency_penalty=0.3, user="u1") == expected
    assert server.requests[1] == server.requests[0]


def test_chat_stream(server: RecordingServer) -> None:
    lite, ours = chat_clients(server)
    server.reply(OK, STREAM_BODY)
    expected = builtins.list(lite.completions_stream(MODEL, MESSAGES))
    server.reply(OK, STREAM_BODY)
    assert builtins.list(ours.completions_stream(MODEL, MESSAGES)) == expected
    assert server.requests[1] == server.requests[0]
    assert json.dumps(server.requests[1].body) == json.dumps(server.requests[0].body)


def test_chat_stream_chunks(server: RecordingServer) -> None:
    _, ours = chat_clients(server)
    server.reply(OK, STREAM_BODY)
    assert builtins.list(ours.completions_stream(MODEL, MESSAGES)) == [{"id": "c1"}, {"id": "c2"}]
    assert [seen.path for seen in server.requests] == ["/chat/completions"]


def test_chat_stream_lazy(server: RecordingServer) -> None:
    _, ours = chat_clients(server)
    server.reply(OK, STREAM_BODY)
    chunks = ours.completions_stream(MODEL, MESSAGES)
    assert server.requests == []
    assert next(chunks) == {"id": "c1"}


def test_chat_completions_request(server: RecordingServer) -> None:
    _, ours = chat_clients(server)
    request = ours.completions(
        MODEL,
        MESSAGES,
        temperature=0.2,
        top_p=0.9,
        n=2,
        max_tokens=32,
        presence_penalty=0.1,
        frequency_penalty=0.3,
        user="u1",
        return_request=True,
    )
    assert request.method == "POST"
    assert request.url == f"{server.url}/chat/completions"
    assert json.dumps(request.json) == (
        '{"model": "gpt-4", "messages": [{"role": "user", "content": "hi"}], '
        '"temperature": 0.2, "top_p": 0.9, "n": 2, "max_tokens": 32, '
        '"presence_penalty": 0.1, "frequency_penalty": 0.3, "user": "u1"}'
    )
    assert request.headers == {
        "Content-Type": "application/json",
        "Authorization": f"Bearer {KEY}",
    }
    slashed = ChatClient(f"{server.url}/", api_key=KEY, timeout=TIMEOUT)
    assert slashed.completions(MODEL, MESSAGES, return_request=True).url == (
        f"{server.url}/chat/completions"
    )
    assert server.requests == []


def test_chat_failures(server: RecordingServer) -> None:
    lite, ours = chat_clients(server)
    server.reply(401, "denied")
    with pytest.raises(LiteLLMUnauthorizedError) as expected:
        lite.completions(MODEL, MESSAGES)
    server.reply(401, "denied")
    with pytest.raises(UnauthorizedError) as actual:
        ours.completions(MODEL, MESSAGES)
    assert str(actual.value) == str(expected.value)
    server.reply(401, "denied")
    with pytest.raises(LiteLLMUnauthorizedError) as expected:
        builtins.list(lite.completions_stream(MODEL, MESSAGES))
    server.reply(401, "denied")
    with pytest.raises(UnauthorizedError) as actual:
        builtins.list(ours.completions_stream(MODEL, MESSAGES))
    assert str(actual.value) == str(expected.value)
    server.reply(500, "boom")
    with pytest.raises(requests.exceptions.HTTPError) as expected:
        lite.completions(MODEL, MESSAGES)
    server.reply(500, "boom")
    with pytest.raises(HTTPError) as actual:
        ours.completions(MODEL, MESSAGES)
    assert str(actual.value) == str(expected.value)
    server.reply(500, "boom")
    with pytest.raises(requests.exceptions.HTTPError) as expected:
        builtins.list(lite.completions_stream(MODEL, MESSAGES))
    server.reply(500, "boom")
    with pytest.raises(HTTPError) as actual:
        builtins.list(ours.completions_stream(MODEL, MESSAGES))
    assert str(actual.value) == str(expected.value)


def test_teams_list(server: RecordingServer) -> None:
    lite, ours = teams_clients(server)
    reply = Reply(OK, "[]")
    compare_call(server, lite.list, ours.list, [], reply)


def test_teams_list_filters(server: RecordingServer) -> None:
    lite, ours = teams_clients(server)
    server.reply(OK, "[]")
    expected = lite.list(user_id="u1", organization_id="o1")
    server.reply(OK, "[]")
    assert ours.list(user_id="u1", organization_id="o1") == expected
    assert server.requests[1] == server.requests[0]


def test_teams_list_empty_filter(server: RecordingServer) -> None:
    lite, ours = teams_clients(server)
    server.reply(OK, "[]")
    expected = lite.list(user_id="", organization_id="o1")
    server.reply(OK, "[]")
    assert ours.list(user_id="", organization_id="o1") == expected
    assert server.requests[1] == server.requests[0]


def test_teams_list_v2(server: RecordingServer) -> None:
    lite, ours = teams_clients(server)
    reply = Reply(OK, '{"teams": []}')
    compare_call(server, lite.list_v2, ours.list_v2, {"teams": []}, reply)


def test_teams_list_v2_filters(server: RecordingServer) -> None:
    lite, ours = teams_clients(server)
    server.reply(OK, '{"teams": []}')
    expected = lite.list_v2(user_id="u1", organization_id="o1", team_id="t1")
    server.reply(OK, '{"teams": []}')
    assert ours.list_v2(user_id="u1", organization_id="o1", team_id="t1") == expected
    assert server.requests[1] == server.requests[0]


def test_teams_list_v2_alias(server: RecordingServer) -> None:
    lite, ours = teams_clients(server)
    server.reply(OK, '{"teams": []}')
    expected = lite.list_v2(team_alias="a1", sort_by="created_at")
    server.reply(OK, '{"teams": []}')
    assert ours.list_v2(team_alias="a1", sort_by="created_at") == expected
    assert server.requests[1] == server.requests[0]


def test_teams_list_v2_paging(server: RecordingServer) -> None:
    lite, ours = teams_clients(server)
    server.reply(OK, '{"teams": []}')
    expected = lite.list_v2(page=2, page_size=5, sort_order="desc")
    server.reply(OK, '{"teams": []}')
    assert ours.list_v2(page=2, page_size=5, sort_order="desc") == expected
    assert server.requests[1] == server.requests[0]


def test_teams_list_v2_empty(server: RecordingServer) -> None:
    lite, ours = teams_clients(server)
    server.reply(OK, '{"teams": []}')
    expected = lite.list_v2(team_alias="")
    server.reply(OK, '{"teams": []}')
    assert ours.list_v2(team_alias="") == expected
    assert server.requests[1] == server.requests[0]


def test_teams_get_available(server: RecordingServer) -> None:
    lite, ours = teams_clients(server)
    reply = Reply(OK, "[]")
    compare_call(server, lite.get_available, ours.get_available, [], reply)
    slashed = TeamsManagementClient(f"{server.url}/", api_key=KEY, timeout=TIMEOUT)
    server.reply(OK, "[]")
    assert slashed.get_available() == []
    assert server.requests[2].path == "/team/available"


def test_teams_available_no_key(server: RecordingServer) -> None:
    lite: LiteTeams = LiteLLMTeamsClient(server.url, timeout=TIMEOUT)
    ours = TeamsManagementClient(server.url, timeout=TIMEOUT)
    compare_call(server, lite.get_available, ours.get_available, [], Reply(OK, "[]"))
    assert server.requests[0].authorization is None
    assert server.requests[0].content_type == "application/json"


def test_teams_failures(server: RecordingServer) -> None:
    lite, ours = teams_clients(server)
    denied = (LiteLLMUnauthorizedError, UnauthorizedError)
    compare_error(server, lite.list, ours.list, denied, Reply(401, "denied"))
    compare_error(server, lite.list_v2, ours.list_v2, denied, Reply(401, "denied"))
    compare_error(server, lite.get_available, ours.get_available, denied, Reply(401, "denied"))
    server.reply(401, "denied")
    with pytest.raises(UnauthorizedError) as actual:
        ours.list()
    assert str(actual.value) == "Authentication failed. Check your API key."
    match actual.value.orig_exception:
        case str() as text:
            assert text == "Authentication failed. Check your API key."
        case HTTPError():
            pytest.fail("the teams 401 error carried the server response")
    failure = (requests.exceptions.HTTPError, HTTPError)
    compare_error(server, lite.list, ours.list, failure, Reply(500, "boom"))
