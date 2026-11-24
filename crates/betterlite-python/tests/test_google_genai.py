import asyncio
import inspect
import json
import logging
from collections.abc import Awaitable, Mapping
from typing import Final, Protocol, TypeAlias, TypedDict

import litellm
import pytest
from genai_server import (
    API_KEY_HEADER,
    BAD_KEY,
    CONFIG,
    CONTENTS,
    EVENTS,
    EXTRA_BODY,
    GENERATE_PATH,
    GOOD_KEY,
    LITELLM,
    LITELLM_ASYNC,
    LITELLM_ERRORS,
    MODEL,
    PREFIXED_MODEL,
    SAFETY_SETTINGS,
    STATUS_BAD_REQUEST,
    STREAM_PATH,
    SYSTEM_INSTRUCTION,
    TOOLS,
    GenaiServer,
    recorded_header,
    reference_dict,
    reference_stream,
    reference_stream_async,
)
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

from betterlite._completion import arguments
from betterlite._completion.errors import UnsupportedCallArgument
from betterlite._endpoints.payload import LOGGER_FN_DROPPED, CallValue
from betterlite._endpoints.proxy import ProxyAuthProvider
from betterlite._genai import arguments as genai_arguments
from betterlite._genai.arguments import fill_settings, session_settings
from betterlite._genai.stream import GenaiStream
from betterlite.exceptions import BadRequestError
from betterlite.google_genai import (
    agenerate_content,
    agenerate_content_stream,
    generate_content,
    generate_content_stream,
)

PROXY_HEADER: Final = "x-proxy-auth"
PROXY_VALUE: Final = "yes"
CLIENT_VALUE: Final = "driver"
VERTEX_PROJECT: Final = "genai-project"
VERTEX_LOCATION: Final = "europe-west4"
CALL_PROJECT: Final = "call-project"
FIRST: Final = 0
SECOND: Final = 1

ContentAnswer: TypeAlias = dict[str, JsonValue] | Awaitable[dict[str, JsonValue]]
StreamAnswer: TypeAlias = GenaiStream | Awaitable[GenaiStream]


class CommonCall(TypedDict):
    model: str
    contents: list[JsonValue]
    api_base: str
    api_key: str


class ErrorPayload(TypedDict):
    type: str
    message: str


class Defaulted(Protocol):
    @property
    def default(self) -> JsonValue: ...


ERROR_PAYLOAD: Final[TypeAdapter[ErrorPayload]] = TypeAdapter(ErrorPayload)


class FakeProxyAuth(ProxyAuthProvider):
    def __init__(self, headers: Mapping[str, str]) -> None:
        self.headers = headers

    @override
    def get_auth_headers(self) -> Mapping[str, str]:
        return self.headers


def common_call(server_url: str, key: str = GOOD_KEY) -> CommonCall:
    return CommonCall(
        model=PREFIXED_MODEL, contents=CONTENTS, api_base=server_url, api_key=key,
    )


# a plain dict, because the reference protocols take **kwargs and reject a TypedDict unpack
def reference_call(server_url: str, key: str = GOOD_KEY) -> dict[str, JsonValue]:
    return {"model": PREFIXED_MODEL, "contents": CONTENTS, "api_base": server_url, "api_key": key}


def signature_rows(signature: inspect.Signature) -> list[str]:
    rows: list[str] = []
    for parameter in signature.parameters.values():
        default: Defaulted = parameter
        rows.append(f"{parameter.name} {parameter.kind.name} {default.default!r}")
    return rows


def content_dict(result: ContentAnswer) -> dict[str, JsonValue]:
    match result:
        case dict() as answer:
            return answer
        case object():
            raise AssertionError("the plain call answered an awaitable")


async def awaited_dict(result: ContentAnswer) -> dict[str, JsonValue]:
    match result:
        case dict():
            raise AssertionError("the flagged call answered a dict")
        case object() as awaitable:
            return await awaitable


def own_stream(result: StreamAnswer) -> GenaiStream:
    match result:
        case GenaiStream() as stream:
            return stream
        case object():
            raise AssertionError("the plain stream call answered an awaitable")


async def awaited_stream(result: StreamAnswer) -> GenaiStream:
    match result:
        case GenaiStream():
            raise AssertionError("the flagged stream call answered a stream")
        case object() as awaitable:
            return await awaitable


def bad_request_payload(failure: BadRequestError) -> ErrorPayload:
    cause = failure.__cause__
    assert cause is not None
    payload = ERROR_PAYLOAD.validate_json(str(cause))
    assert failure.status_code == STATUS_BAD_REQUEST
    assert "API key not valid" in payload["message"]
    return payload


async def async_content(server_url: str) -> tuple[dict[str, JsonValue], dict[str, JsonValue]]:
    ours = await agenerate_content(**common_call(server_url))
    reference = reference_dict(await LITELLM_ASYNC.agenerate_content(**reference_call(server_url)))
    return ours, reference


async def flagged_content(server_url: str) -> tuple[dict[str, JsonValue], dict[str, JsonValue]]:
    ours = generate_content(**common_call(server_url), agenerate_content=True)
    reference = LITELLM_ASYNC.generate_content(
        **reference_call(server_url), agenerate_content=True,
    )
    return await awaited_dict(ours), reference_dict(await reference)


async def async_streams(server_url: str) -> tuple[list[bytes], list[bytes]]:
    ours = [chunk async for chunk in await agenerate_content_stream(**common_call(server_url))]
    reference = [
        chunk
        async for chunk in await reference_stream_async(
            LITELLM_ASYNC.agenerate_content_stream(**reference_call(server_url))
        )
    ]
    return ours, reference


async def flagged_stream(server_url: str) -> tuple[list[bytes], list[bytes]]:
    ours = generate_content_stream(**common_call(server_url), agenerate_content_stream=True)
    reference = LITELLM_ASYNC.generate_content_stream(
        **reference_call(server_url), agenerate_content_stream=True,
    )
    return (
        [chunk async for chunk in await awaited_stream(ours)],
        [chunk async for chunk in await reference_stream_async(reference)],
    )


async def close_async_stream(server_url: str) -> None:
    stream = await agenerate_content_stream(**common_call(server_url))
    assert len([chunk async for chunk in stream]) == len(EVENTS)
    await stream.aclose()
    await stream.aclose()
    with pytest.raises(StopAsyncIteration):
        await anext(stream)


async def reference_bad_stream(server_url: str) -> None:
    await LITELLM_ASYNC.agenerate_content_stream(**reference_call(server_url, BAD_KEY))


def test_signature_matches_litellm() -> None:
    assert signature_rows(inspect.signature(generate_content)) == signature_rows(
        inspect.signature(LITELLM.generate_content)
    )
    assert signature_rows(inspect.signature(agenerate_content)) == signature_rows(
        inspect.signature(LITELLM.agenerate_content)
    )
    assert signature_rows(inspect.signature(generate_content_stream)) == signature_rows(
        inspect.signature(LITELLM.generate_content_stream)
    )
    assert signature_rows(inspect.signature(agenerate_content_stream)) == signature_rows(
        inspect.signature(LITELLM.agenerate_content_stream)
    )


def test_plain_matches_litellm() -> None:
    with GenaiServer() as server:
        ours = content_dict(generate_content(**common_call(server.url)))
        reference = reference_dict(LITELLM.generate_content(**reference_call(server.url)))
        paths = [call["path"] for call in server.calls]
        bodies = [call["body"] for call in server.calls]
        key = recorded_header(server, FIRST, API_KEY_HEADER)
        reference_key = recorded_header(server, SECOND, API_KEY_HEADER)
    assert ours == reference
    assert paths == [GENERATE_PATH, GENERATE_PATH]
    assert bodies[FIRST] == bodies[SECOND]
    assert bodies[FIRST]["model"] == MODEL
    assert bodies[FIRST]["contents"] == CONTENTS
    assert key == GOOD_KEY
    assert reference_key == GOOD_KEY


def test_optional_matches_litellm() -> None:
    with GenaiServer() as server:
        ours = content_dict(
            generate_content(
                **common_call(server.url),
                config=CONFIG,
                tools=TOOLS,
                extra_body=EXTRA_BODY,
                systemInstruction=SYSTEM_INSTRUCTION,
                safetySettings=SAFETY_SETTINGS,
            )
        )
        reference = reference_dict(
            LITELLM.generate_content(
                **reference_call(server.url),
                config=CONFIG,
                tools=TOOLS,
                extra_body=EXTRA_BODY,
                systemInstruction=SYSTEM_INSTRUCTION,
                safetySettings=SAFETY_SETTINGS,
            )
        )
        bodies = [call["body"] for call in server.calls]
    assert ours == reference
    assert bodies[FIRST] == bodies[SECOND]
    assert bodies[FIRST]["generationConfig"] == CONFIG
    assert bodies[FIRST]["systemInstruction"] == SYSTEM_INSTRUCTION
    assert bodies[FIRST]["safetySettings"] == SAFETY_SETTINGS
    assert bodies[FIRST]["labels"] == EXTRA_BODY["labels"]
    assert bodies[FIRST]["tools"] == TOOLS


def test_optional_module_api_key(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("GOOGLE_API_KEY", raising=False)
    monkeypatch.delenv("GEMINI_API_KEY", raising=False)
    expected: dict[str, str] = {"api_key": GOOD_KEY}
    assert session_settings() == "{}"
    their_key = litellm.api_key
    try:
        litellm.api_key = GOOD_KEY
        genai_arguments.settings_seam["api_key"] = GOOD_KEY
        assert session_settings() == json.dumps(expected)
        with GenaiServer() as server:
            answers = [
                content_dict(
                    generate_content(model=PREFIXED_MODEL, contents=CONTENTS, api_base=server.url)
                ),
                reference_dict(
                    LITELLM.generate_content(
                        model=PREFIXED_MODEL, contents=CONTENTS, api_base=server.url
                    )
                ),
            ]
            keys = [recorded_header(server, index, API_KEY_HEADER) for index in (FIRST, SECOND)]
    finally:
        litellm.api_key = their_key
        genai_arguments.settings_seam.clear()
    assert answers[FIRST] == answers[SECOND]
    assert keys == [GOOD_KEY, GOOD_KEY]


def test_optional_vertex_settings() -> None:
    genai_arguments.settings_seam["vertex_project"] = VERTEX_PROJECT
    genai_arguments.settings_seam["vertex_location"] = VERTEX_LOCATION
    try:
        forwarded: dict[str, CallValue] = {"api_base": None, "vertex_location": None}
        fill_settings(forwarded)
        explicit: dict[str, CallValue] = {"vertex_project": CALL_PROJECT, "vertex_location": None}
        fill_settings(explicit)
        empty: dict[str, CallValue] = {"vertex_project": ""}
        fill_settings(empty)
    finally:
        genai_arguments.settings_seam.clear()
    assert forwarded == {
        "api_base": None,
        "vertex_project": VERTEX_PROJECT,
        "vertex_location": VERTEX_LOCATION,
    }
    assert explicit == {"vertex_project": CALL_PROJECT, "vertex_location": VERTEX_LOCATION}
    assert empty == {"vertex_project": ""}


def test_optional_proxy_headers() -> None:
    arguments.proxy_auth = FakeProxyAuth({PROXY_HEADER: PROXY_VALUE})
    try:
        with GenaiServer() as server:
            ours = content_dict(generate_content(**common_call(server.url)))
            header = recorded_header(server, FIRST, PROXY_HEADER)
    finally:
        arguments.proxy_auth = None
    assert header == PROXY_VALUE
    assert ours["modelVersion"] == MODEL


def test_async_matches_litellm() -> None:
    with GenaiServer() as server:
        ours, reference = asyncio.run(async_content(server.url))
        paths = [call["path"] for call in server.calls]
        bodies = [call["body"] for call in server.calls]
    assert ours == reference
    assert paths == [GENERATE_PATH, GENERATE_PATH]
    assert bodies[FIRST] == bodies[SECOND]


def test_coroutine_flag_matches() -> None:
    with GenaiServer() as server:
        ours, reference = asyncio.run(flagged_content(server.url))
        paths = [call["path"] for call in server.calls]
    assert ours == reference
    assert paths == [GENERATE_PATH, GENERATE_PATH]


def test_stream_matches_litellm() -> None:
    with GenaiServer() as server:
        ours = list(own_stream(generate_content_stream(**common_call(server.url))))
        reference = list(
            reference_stream(LITELLM.generate_content_stream(**reference_call(server.url)))
        )
        paths = [call["path"] for call in server.calls]
    assert ours == reference
    assert len(ours) == len(EVENTS)
    assert paths == [STREAM_PATH, STREAM_PATH]
    assert ours[FIRST].startswith(b"data: ")
    assert ours[FIRST].endswith(b"\n\n")


def test_stream_close_after_end() -> None:
    with GenaiServer() as server:
        stream = own_stream(generate_content_stream(**common_call(server.url)))
        assert len(list(stream)) == len(EVENTS)
        with pytest.raises(StopIteration):
            next(stream)
        stream.close()
        stream.close()
        with pytest.raises(StopIteration):
            next(stream)


def test_stream_error_class() -> None:
    with GenaiServer() as server:
        with pytest.raises(BadRequestError) as caught:
            generate_content_stream(**common_call(server.url, BAD_KEY))
        with pytest.raises(BadRequestError) as awaited:
            asyncio.run(agenerate_content_stream(**common_call(server.url, BAD_KEY)))
        with pytest.raises(LITELLM_ERRORS.BadRequestError):
            LITELLM.generate_content_stream(**reference_call(server.url, BAD_KEY))
        with pytest.raises(LITELLM_ERRORS.BadRequestError):
            asyncio.run(reference_bad_stream(server.url))
        paths = [call["path"] for call in server.calls]
    assert bad_request_payload(caught.value)["type"] == "BadRequestError"
    assert bad_request_payload(awaited.value)["type"] == "BadRequestError"
    assert paths == [STREAM_PATH, STREAM_PATH, STREAM_PATH, STREAM_PATH]


def test_async_stream_matches() -> None:
    with GenaiServer() as server:
        ours, reference = asyncio.run(async_streams(server.url))
        bodies = [call["body"] for call in server.calls]
    assert ours == reference
    assert len(ours) == len(EVENTS)
    assert bodies[FIRST]["contents"] == CONTENTS


def test_coroutine_stream_flags() -> None:
    with GenaiServer() as server:
        ours, reference = asyncio.run(flagged_stream(server.url))
        paths = [call["path"] for call in server.calls]
    assert ours == reference
    assert paths == [STREAM_PATH, STREAM_PATH]


def test_stream_close_async() -> None:
    with GenaiServer() as server:
        asyncio.run(close_async_stream(server.url))


def test_error_class_and_cause() -> None:
    with GenaiServer() as server:
        with pytest.raises(BadRequestError) as caught:
            generate_content(**common_call(server.url, BAD_KEY))
        with pytest.raises(LITELLM_ERRORS.BadRequestError):
            LITELLM.generate_content(**reference_call(server.url, BAD_KEY))
    payload = bad_request_payload(caught.value)
    assert payload["type"] == "BadRequestError"


def test_client_is_refused(caplog: pytest.LogCaptureFixture) -> None:
    with GenaiServer() as server:
        with pytest.raises(UnsupportedCallArgument) as caught:
            generate_content(**common_call(server.url), client=CLIENT_VALUE)
        with pytest.raises(UnsupportedCallArgument) as session:
            generate_content(**common_call(server.url), shared_session=CLIENT_VALUE)
        with caplog.at_level(logging.WARNING, logger="LiteLLM"):
            logged = content_dict(
                generate_content(**common_call(server.url), logger_fn=CLIENT_VALUE)
            )
        paths = [call["path"] for call in server.calls]
    assert caught.value.argument == "client"
    assert session.value.argument == "shared_session"
    assert logged["modelVersion"] == MODEL
    assert LOGGER_FN_DROPPED in caplog.messages
    assert paths == [GENERATE_PATH]


def test_adapter_model_error() -> None:
    values = common_call("http://127.0.0.1:9")
    values["model"] = "openai/gpt-4o"
    with pytest.raises(Exception, match="chat completion adapter"):
        generate_content(**values)
