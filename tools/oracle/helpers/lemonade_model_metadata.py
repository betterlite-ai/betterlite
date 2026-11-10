import json
from typing import Literal, NotRequired, TypeAlias, TypedDict, cast
from unittest.mock import patch

import httpx
import litellm
from litellm.llms.lemonade.chat import transformation
from litellm.llms.lemonade.chat.transformation import LemonadeChatConfig

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Reply(TypedDict):
    status: int
    body: NotRequired[Json]
    raw: NotRequired[str]


class OriginQuery(TypedDict):
    api_base: NotRequired[str | None]
    api_key: NotRequired[str | None]
    environment: NotRequired[dict[str, str]]
    global_keys: NotRequired[dict[str, str]]


class ModelInfoQuery(TypedDict):
    model: str
    api_base: NotRequired[str | None]
    api_key: NotRequired[str | None]
    environment: NotRequired[dict[str, str]]
    global_keys: NotRequired[dict[str, str]]
    reply: NotRequired[Reply | None]


class Origin(TypedDict):
    operation: Literal["origin"]
    queries: list[OriginQuery]


class PositiveInt(TypedDict):
    operation: Literal["positive_int"]
    values: list[Json]
    model_infos: NotRequired[list[dict[str, Json]]]


class ProviderEntry(TypedDict):
    operation: Literal["provider_entry"]
    model_infos: list[dict[str, Json]]


class ModelInfo(TypedDict):
    operation: Literal["model_info"]
    queries: list[ModelInfoQuery]


Input: TypeAlias = Origin | PositiveInt | ProviderEntry | ModelInfo


class _Secrets:
    def __init__(self, environment: dict[str, str]) -> None:
        self._environment = environment

    def lookup(self, name: str) -> str | None:
        return self._environment.get(name)


class _ReplyClient:
    def __init__(self, reply: Reply | None) -> None:
        self._reply = reply
        self.seen: list[tuple[str, str | None]] = []

    def get(self, *, url: str, headers: dict[str, str] | None = None) -> httpx.Response:
        authorization = None if headers is None else headers.get("Authorization")
        self.seen.append((url, authorization))
        if self._reply is None:
            raise httpx.ConnectError("fixture transport failure")
        return httpx.Response(
            self._reply["status"],
            content=_reply_bytes(self._reply),
            request=httpx.Request("GET", url),
        )


def _reply_bytes(reply: Reply) -> bytes:
    raw = reply.get("raw")
    if raw is not None:
        return raw.encode()
    body = reply.get("body")
    if body is None:
        return b""
    return json.dumps(body).encode()


def _json_value(value: object) -> Json:
    return cast(Json, json.loads(json.dumps(value)))


def _apply_globals(global_keys: dict[str, str]) -> None:
    litellm.lemonade_key = global_keys.get("lemonade_key")


def _origin_answer(query: OriginQuery) -> Json:
    config = LemonadeChatConfig()
    _apply_globals(query.get("global_keys", {}))
    secrets = _Secrets(query.get("environment", {}))
    with patch.object(transformation, "get_secret_str", secrets.lookup):
        base, key = config._get_openai_compatible_provider_info(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            api_base=query.get("api_base"), api_key=query.get("api_key")
        )
        headers = config._get_auth_headers(key)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]
    return {
        "api_base": base,
        "api_key": key,
        "headers": _json_value(headers),  # pyright: ignore[reportUnknownArgumentType]
    }


def _positive_int_answer(call: PositiveInt) -> Json:
    config = LemonadeChatConfig()
    values = [
        LemonadeChatConfig._get_positive_int(value)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        for value in call["values"]
    ]
    context_windows: list[Json] = [
        cast(Json, config._get_context_window(model_info))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
        for model_info in call.get("model_infos", [])
    ]
    return {"values": cast(Json, values), "context_windows": context_windows}


def _provider_entry_answer(call: ProviderEntry) -> Json:
    config = LemonadeChatConfig()
    entries: list[Json] = [
        cast(Json, json.loads(json.dumps(config._get_provider_specific_entry(model_info))))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
        for model_info in call["model_infos"]
    ]
    return {"entries": entries, "model_infos": cast(Json, call["model_infos"])}


def _model_info_answer(query: ModelInfoQuery) -> Json:
    config = LemonadeChatConfig()
    _apply_globals(query.get("global_keys", {}))
    client = _ReplyClient(query.get("reply"))
    secrets = _Secrets(query.get("environment", {}))
    with (
        patch.object(transformation, "get_secret_str", secrets.lookup),
        patch.object(litellm, "module_level_client", client),
    ):
        info = config.get_model_info(
            model=query["model"],
            api_base=query.get("api_base"),
            api_key=query.get("api_key"),
        )
    url, authorization = client.seen[0]
    return {
        "info": cast(Json, json.loads(json.dumps(info))),
        "url": url,
        "authorization": authorization,
    }


def run(input_value: Input) -> Json:
    if input_value["operation"] == "origin":
        return cast(Json, [_origin_answer(query) for query in input_value["queries"]])
    if input_value["operation"] == "positive_int":
        return _positive_int_answer(input_value)
    if input_value["operation"] == "provider_entry":
        return _provider_entry_answer(input_value)
    if input_value["operation"] == "model_info":
        return cast(Json, [_model_info_answer(query) for query in input_value["queries"]])
    raise ValueError(f"unknown lemonade model metadata operation {input_value['operation']}")
