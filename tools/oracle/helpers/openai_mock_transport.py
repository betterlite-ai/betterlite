import asyncio
import time
import uuid
from typing import TypeAlias
from unittest import mock

import aiohttp
import httpx
import litellm
from litellm.llms.custom_httpx.mock_transport import (
    MockOpenAITransport,
    _chat_completion_json,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]
)
from litellm.llms.openai.common_utils import BaseOpenAILLM

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _reply_literals(input_value: dict[str, Json]) -> Json:
    if set(input_value) != {"case", "model", "clock", "uuid"}:
        raise ValueError("mock_reply_literals input fields are case, model, clock, and uuid")
    model = input_value["model"]
    clock = input_value["clock"]
    identifier = input_value["uuid"]
    if not isinstance(model, str) or not isinstance(clock, int) or not isinstance(identifier, str):
        raise ValueError("model and uuid must be strings and clock must be an int")
    with (
        mock.patch.object(time, "time", return_value=float(clock)),
        mock.patch.object(uuid, "uuid4", return_value=uuid.UUID(identifier)),
    ):
        return _chat_completion_json(model)  # pyright: ignore[reportUnknownVariableType]


def _probe_outcome(content: str) -> Json:
    request = httpx.Request("POST", "http://mock.test/v1/completions", content=content.encode())
    try:
        model, stream = MockOpenAITransport._parse_request(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            request
        )
    except AttributeError as error:
        return {"error": str(error)}
    return {"model": model, "stream": stream}


def _request_decoder(input_value: dict[str, Json]) -> Json:
    if set(input_value) != {"case", "probes"}:
        raise ValueError("mock_request_decoder input fields are case and probes")
    probes = input_value["probes"]
    if not isinstance(probes, list):
        raise ValueError("probes must be a list")
    outcomes: list[Json] = []
    for probe in probes:
        if not isinstance(probe, dict) or set(probe) != {"content"}:
            raise ValueError("each probe needs exactly content")
        content = probe["content"]
        if not isinstance(content, str):
            raise ValueError("content must be a string")
        outcomes.append(_probe_outcome(content))
    return outcomes


def _classify(client: httpx.AsyncClient | httpx.Client, supplied: object | None) -> str:
    if supplied is not None and client is supplied:
        return "supplied"
    transport = client._transport  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    if isinstance(transport, MockOpenAITransport):
        return "mock"
    return "default"


async def _async_client(
    *, network_mock: bool, supplied_kind: str
) -> tuple[httpx.AsyncClient | None, httpx.AsyncClient | None]:
    supplied = httpx.AsyncClient() if supplied_kind == "client" else None
    shared = aiohttp.ClientSession() if supplied_kind == "shared" else None
    with (
        mock.patch.object(litellm, "network_mock", network_mock),
        mock.patch.object(litellm, "aclient_session", supplied),
    ):
        client = BaseOpenAILLM._get_async_http_client(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            shared_session=shared
        )
    return client, supplied


def _sync_client(
    *, network_mock: bool, supplied_kind: str
) -> tuple[httpx.Client | None, httpx.Client | None]:
    supplied = httpx.Client() if supplied_kind == "client" else None
    with (
        mock.patch.object(litellm, "network_mock", network_mock),
        mock.patch.object(litellm, "client_session", supplied),
    ):
        client = BaseOpenAILLM._get_sync_http_client()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return client, supplied


def _selection_outcome(scenario: Json) -> str:
    expected = {"kind", "network_mock", "supplied"}
    if not isinstance(scenario, dict) or set(scenario) != expected:
        raise ValueError("each selection scenario needs kind, network_mock, and supplied")
    kind = scenario["kind"]
    network_mock = scenario["network_mock"]
    supplied_kind = scenario["supplied"]
    if kind not in ("async", "sync"):
        raise ValueError("kind must be async or sync")
    if not isinstance(network_mock, bool):
        raise ValueError("network_mock must be a bool")
    if supplied_kind not in ("none", "client", "shared"):
        raise ValueError("supplied must be none, client, or shared")
    if kind == "async":
        client, supplied = asyncio.run(
            _async_client(network_mock=network_mock, supplied_kind=supplied_kind)
        )
    else:
        client, supplied = _sync_client(network_mock=network_mock, supplied_kind=supplied_kind)
    if client is None:
        raise ValueError("expected a mock-selected http client but got None")
    return _classify(client, supplied)


def _selection(input_value: dict[str, Json]) -> Json:
    if set(input_value) != {"case", "scenarios"}:
        raise ValueError("mock_selection input fields are case and scenarios")
    scenarios = input_value["scenarios"]
    if not isinstance(scenarios, list):
        raise ValueError("scenarios must be a list")
    return [_selection_outcome(scenario) for scenario in scenarios]


def run(input_value: dict[str, Json]) -> Json:
    case = input_value.get("case")
    if case == "mock_reply_literals":
        return _reply_literals(input_value)
    if case == "mock_request_decoder":
        return _request_decoder(input_value)
    if case == "mock_selection":
        return _selection(input_value)
    raise ValueError(f"unknown openai_mock_transport case: {case}")
