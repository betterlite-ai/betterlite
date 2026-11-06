import asyncio
import os
import platform as platform_module
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import TypeAlias, TypedDict, cast
from unittest.mock import patch

from litellm.llms.chatgpt import common_utils
from litellm.llms.chatgpt.authenticator import Authenticator
from litellm.llms.chatgpt.chat.streaming_utils import ChatGPTToolCallNormalizer
from litellm.llms.chatgpt.chat.transformation import ChatGPTConfig
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import ModelResponseStream

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]

SESSION_ID = "00000000-0000-4000-8000-000000000000"
TOP_LEVEL_KEYS = ("id", "object", "created", "model", "system_fingerprint", "choices")


class Input(TypedDict):
    environment: dict[str, str]
    platform: dict[str, str]
    record: str | None
    now: float
    step: Json


def obj(value: Json) -> Object:
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return value


def text(fields: Object, name: str) -> str:
    value = fields.get(name)
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def optional_text(fields: Object, name: str) -> str | None:
    value = fields.get(name)
    if value is None or isinstance(value, str):
        return value
    raise TypeError(f"{name} must be a string or null")


def array(fields: Object, name: str) -> list[Json]:
    value = fields.get(name)
    if not isinstance(value, list):
        raise TypeError(f"{name} must be an array")
    return value


def compact(value: Json) -> Json:
    if isinstance(value, dict):
        return {key: compact(item) for key, item in value.items() if item is not None}
    if isinstance(value, list):
        return [compact(item) for item in value]
    return value


def view(chunk: ModelResponseStream) -> Json:
    dumped = cast(Object, chunk.model_dump())  # pyright: ignore[reportUnknownMemberType]
    return compact({key: dumped[key] for key in TOP_LEVEL_KEYS if key in dumped})


class Source:
    def __init__(self, chunks: list[Json]) -> None:
        self.chunks: list[Json] = list(chunks)

    def __iter__(self) -> "Source":
        return self

    def __aiter__(self) -> "Source":
        return self

    def __next__(self) -> ModelResponseStream:
        if not self.chunks:
            raise StopIteration
        chunk = obj(self.chunks.pop(0))
        return ModelResponseStream(**chunk)  # pyright: ignore[reportArgumentType, reportUnknownMemberType]

    async def __anext__(self) -> ModelResponseStream:
        if not self.chunks:
            raise StopAsyncIteration
        chunk = obj(self.chunks.pop(0))
        return ModelResponseStream(**chunk)  # pyright: ignore[reportArgumentType, reportUnknownMemberType]


def collect_sync(chunks: list[Json]) -> list[Json]:
    normalizer = ChatGPTToolCallNormalizer(Source(chunks))
    return [view(chunk) for chunk in normalizer]


async def collect_async(chunks: list[Json]) -> list[Json]:
    normalizer = ChatGPTToolCallNormalizer(Source(chunks))
    return [view(chunk) async for chunk in normalizer]


class Recorder(Authenticator):
    def __init__(self) -> None:
        super().__init__()
        self.calls: list[str] = []

    def get_api_base(self) -> str:
        self.calls.append("get_api_base")
        return super().get_api_base()

    def get_access_token(self) -> str:
        self.calls.append("get_access_token")
        return super().get_access_token()

    def get_account_id(self) -> str | None:
        self.calls.append("get_account_id")
        return super().get_account_id()


def step_provider_info(config: ChatGPTConfig, spec: Object) -> Object:
    api_base = config.api_base_without_login()
    result = config._get_openai_compatible_provider_info(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
        text(spec, "model"),
        optional_text(spec, "api_base"),
        optional_text(spec, "api_key"),
        text(spec, "provider"),
    )
    provider_base, api_key, provider = result
    return {
        "api_base": api_base,
        "provider_api_base": provider_base,
        "api_key": api_key,
        "provider": provider,
    }


def step_headers(config: ChatGPTConfig, spec: Object) -> Object:
    return cast(
        Object,
        config.validate_environment(  # pyright: ignore[reportUnknownMemberType]
            headers=cast(dict[str, str], spec.get("headers") or {}),
            model="gpt-5",
            messages=cast(list[AllMessageValues], []),
            optional_params={},
            litellm_params=obj(spec.get("litellm_params") or {}),
            api_key=optional_text(spec, "api_key"),
            api_base=None,
        ),
    )


def step_params(config: ChatGPTConfig, spec: Object) -> Object:
    model = text(spec, "model")
    outputs: list[Json] = []
    for requested in array(spec, "requests"):
        optional: Object = {}
        config.map_openai_params(  # pyright: ignore[reportUnknownMemberType]
            obj(requested), optional, model, drop_params=False
        )
        outputs.append(optional)
    return {"params": outputs}


def step_normalize(spec: Object) -> Object:
    chunks = array(spec, "chunks")
    return {"sync": collect_sync(chunks), "async": asyncio.run(collect_async(chunks))}


def dispatch(config: ChatGPTConfig, step: Json) -> Object:
    match step:
        case {"provider_info": spec}:
            return step_provider_info(config, obj(spec))
        case {"headers": spec}:
            return step_headers(config, obj(spec))
        case {"params": spec}:
            return step_params(config, obj(spec))
        case {"normalize": spec}:
            return step_normalize(obj(spec))
        case _:
            raise ValueError(f"unknown chatgpt_chat step: {step}")


def run(input_value: Input) -> Json:
    with TemporaryDirectory() as root:
        record = input_value["record"]
        if record is not None:
            _ = (Path(root) / "auth.json").write_text(record)
        environment = dict(input_value["environment"])
        environment["CHATGPT_TOKEN_DIR"] = root
        environment["CHATGPT_AUTH_FILE"] = "auth.json"
        pinned = input_value["platform"]
        with (
            patch.dict(os.environ, environment),
            patch.object(platform_module, "system", return_value=pinned["system"]),
            patch.object(platform_module, "release", return_value=pinned["release"]),
            patch.object(platform_module, "machine", return_value=pinned["machine"]),
            patch.object(common_utils, "uuid4", return_value=SESSION_ID),
            patch("litellm.llms.chatgpt.authenticator.time.time", return_value=input_value["now"]),
        ):
            config = ChatGPTConfig()
            recorder = Recorder()
            config.authenticator = recorder
            value = dispatch(config, input_value["step"])
        value["authentication_calls"] = cast(Json, recorder.calls)
        return value
