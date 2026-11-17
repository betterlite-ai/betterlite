import asyncio
import os
from collections.abc import Coroutine
from typing import Final, Literal, NotRequired, Protocol, TypeAlias, TypedDict, runtime_checkable

import litellm
from litellm.exceptions import BadRequestError
from litellm.llms.openai.chat import gpt_transformation
from litellm.llms.openai.chat.gpt_transformation import OpenAIGPTConfig
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

Mode: TypeAlias = Literal["sync", "async_base_class", "async_subclass"]

# the alias pins the exception type, so raising it and catching it need no suppression
REJECTION: Final[type[Exception]] = BadRequestError
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class Request(TypedDict):
    model: str
    messages: list[JsonValue]
    optional_params: dict[str, JsonValue]
    system_messages_first: bool
    environment: dict[str, str]
    mode: Mode
    provider: NotRequired[str | None]
    api_base: NotRequired[str | None]
    global_api_base: NotRequired[str | None]
    urls: NotRequired[dict[str, str]]
    refuse: NotRequired[bool]


class TransformRequests(TypedDict):
    requests: list[Request]


class Input(TypedDict):
    transform_requests: TransformRequests


@runtime_checkable
class TransformConfig(Protocol):
    def transform_request(
        self,
        model: str,
        messages: list[JsonValue],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        headers: dict[str, JsonValue],
    ) -> dict[str, JsonValue]: ...

    def async_transform_request(
        self,
        model: str,
        messages: list[JsonValue],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        headers: dict[str, JsonValue],
    ) -> Coroutine[JsonValue, JsonValue, dict[str, JsonValue]]: ...


class Bound(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    config: TransformConfig


class Rejection(BaseModel):
    status_code: int


# the async base class branch answers first, so only it keeps _is_base_class
class BaseClassConfig(OpenAIGPTConfig):
    _is_base_class = True

    def __init__(self) -> None:
        pass


class MappedConvert:
    def __init__(self, urls: dict[str, str]) -> None:
        self.urls = urls

    def __call__(self, url: str) -> str:
        data_url = self.urls.get(url)
        if data_url is not None:
            return data_url
        raise REJECTION(
            f"Unable to fetch the request media URL from the case answers. url={url}",
            None,
            "openai",
        )


class AwaitedConvert:
    def __init__(self, convert: MappedConvert) -> None:
        self.convert = convert

    async def __call__(self, url: str) -> str:
        return self.convert(url)


INPUT: Final[TypeAdapter[Input]] = TypeAdapter(Input)


def refused(request: Request) -> bool:
    value = request.get("refuse")
    if value is None:
        return False
    return value


def mapping(request: Request) -> dict[str, str]:
    urls = request.get("urls")
    if urls is None:
        return {}
    return urls


def apply_environment(variables: dict[str, str]) -> None:
    os.environ.pop("OPENAI_BASE_URL", None)
    os.environ.pop("OPENAI_API_BASE", None)
    os.environ.update(variables)


def bound(instance: OpenAIGPTConfig) -> TransformConfig:
    holder: dict[str, OpenAIGPTConfig] = {"config": instance}
    return Bound.model_validate(holder).config


def config_for(mode: Mode) -> TransformConfig:
    if mode == "async_base_class":
        return bound(BaseClassConfig())
    return bound(OpenAIGPTConfig())


def body_for(request: Request, config: TransformConfig) -> dict[str, JsonValue]:
    litellm_params: dict[str, JsonValue] = {
        "custom_llm_provider": request.get("provider"),
        "api_base": request.get("api_base"),
    }
    optional_params: dict[str, JsonValue] = dict(request["optional_params"])
    if request["mode"] == "sync":
        return config.transform_request(
            model=request["model"],
            messages=request["messages"],
            optional_params=optional_params,
            litellm_params=litellm_params,
            headers={},
        )
    return asyncio.run(
        config.async_transform_request(
            model=request["model"],
            messages=request["messages"],
            optional_params=optional_params,
            litellm_params=litellm_params,
            headers={},
        )
    )


def transform(request: Request) -> JsonValue:
    litellm.openai_system_messages_first = request["system_messages_first"]
    litellm.api_base = request.get("global_api_base")
    apply_environment(request["environment"])
    convert = MappedConvert({} if refused(request) else mapping(request))
    object.__setattr__(gpt_transformation, "convert_url_to_base64", convert)
    object.__setattr__(gpt_transformation, "async_convert_url_to_base64", AwaitedConvert(convert))
    body = body_for(request, config_for(request["mode"]))
    return JSON.validate_python(body)


def outcome(request: Request) -> dict[str, JsonValue]:
    try:
        return {"ok": True, "value": transform(request)}
    except REJECTION as error:
        rejection = Rejection.model_validate(error, from_attributes=True)
        return {"ok": False, "status": rejection.status_code}


def run(input_value: JsonValue) -> JsonValue:
    parsed = INPUT.validate_python(input_value)
    requests = parsed["transform_requests"]["requests"]
    outcomes: list[JsonValue] = []
    for request in requests:
        outcomes.append(outcome(request))
    return JSON.validate_python(outcomes)
