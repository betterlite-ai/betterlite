import asyncio
import datetime as dt
import inspect
from collections.abc import Awaitable
from os import PathLike
from pathlib import Path
from typing import Final, Literal, NotRequired, Protocol, TypedDict, Unpack, runtime_checkable

import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.videos.transformation import BaseVideoConfig
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.types.router import GenericLiteLLMParams
from litellm.types.videos.main import CharacterObject
from litellm.videos import main as videos_main
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

litellm.suppress_debug_info = True

JSON: Final = TypeAdapter(JsonValue)
CONFIG: Final = ConfigDict(arbitrary_types_allowed=True)
START: Final = dt.datetime(2024, 1, 1, tzinfo=dt.UTC)
CHARACTER_ID: Final = "char-1"
CHARACTER_CREATED_AT: Final = 1_700_000_000


class SharedInput(TypedDict):
    asynchronous: bool
    litellm_call_id: str
    timeout: NotRequired[float | None]
    custom_llm_provider: NotRequired[str | None]
    extra_headers: NotRequired[dict[str, str] | None]
    extra: NotRequired[dict[str, JsonValue]]


class CreateInput(SharedInput):
    function: Literal["create"]
    name: str
    video: str


class GetInput(SharedInput):
    function: Literal["get"]
    character_id: str


Input = CreateInput | GetInput
Created = CharacterObject | Awaitable[CharacterObject]
Arg = JsonValue | dict[str, str]


@runtime_checkable
class CharacterApi(Protocol):
    # LiteLLM leaves **kwargs untyped here; this view pins the arguments the calls pass
    def video_create_character(
        self, name: str, video: PathLike[str], *, litellm_logging_obj: Logging, **kwargs: Arg
    ) -> Created: ...
    def avideo_create_character(
        self, name: str, video: PathLike[str], *, litellm_logging_obj: Logging, **kwargs: Arg
    ) -> Created: ...
    def video_get_character(
        self, character_id: str, *, litellm_logging_obj: Logging, **kwargs: Arg
    ) -> Created: ...
    def avideo_get_character(
        self, character_id: str, *, litellm_logging_obj: Logging, **kwargs: Arg
    ) -> Created: ...


class SharedHandlerArguments(TypedDict):
    video_provider_config: BaseVideoConfig
    client: HTTPHandler | AsyncHTTPHandler | None
    custom_llm_provider: str
    litellm_params: GenericLiteLLMParams
    logging_obj: Logging
    extra_headers: dict[str, str] | None
    timeout: float | None
    _is_async: bool


class CreateHandlerArguments(SharedHandlerArguments):
    name: str
    video: PathLike[str]


class GetHandlerArguments(SharedHandlerArguments):
    character_id: str


class CreateCharacterHandler(Protocol):
    def __call__(self, **kwargs: Unpack[CreateHandlerArguments]) -> CharacterObject: ...


class GetCharacterHandler(Protocol):
    def __call__(self, **kwargs: Unpack[GetHandlerArguments]) -> CharacterObject: ...


@runtime_checkable
class HandlerFields(Protocol):
    video_create_character_handler: CreateCharacterHandler
    video_get_character_handler: GetCharacterHandler


@runtime_checkable
class VideosFields(Protocol):
    base_llm_http_handler: HandlerFields


API: Final = TypeAdapter(CharacterApi, config=CONFIG).validate_python(videos_main)
VIDEOS: Final = TypeAdapter(VideosFields, config=CONFIG).validate_python(videos_main)


class RecordedCall(BaseModel):
    function: Literal["create", "get"]
    provider: str
    timeout: float
    asynchronous: bool
    api_base: str | None
    api_key: str | None
    extra_headers: dict[str, str] | None
    name: str | None
    video: str | None
    character_id: str | None


class RecordedLogging(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    model: str | None = None
    call_type: str
    user: str | None = None
    custom_llm_provider: str | None = None
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]


class Output(BaseModel):
    result: JsonValue
    calls: list[RecordedCall]
    logging: RecordedLogging


class Recorder:
    def __init__(self) -> None:
        self.calls: list[RecordedCall] = []
        self.logging: RecordedLogging | None = None

    def video_create_character_handler(
        self, **kwargs: Unpack[CreateHandlerArguments]
    ) -> CharacterObject:
        recorded = recorded_call(kwargs, "create", kwargs["name"], str(kwargs["video"]))
        return self.record(recorded, kwargs["logging_obj"])

    def video_get_character_handler(
        self, **kwargs: Unpack[GetHandlerArguments]
    ) -> CharacterObject:
        recorded = recorded_call(kwargs, "get", kwargs["character_id"])
        return self.record(recorded, kwargs["logging_obj"])

    def record(self, call: RecordedCall, logging_obj: Logging) -> CharacterObject:
        self.calls.append(call)
        self.logging = RecordedLogging.model_validate(logging_obj)
        return CharacterObject(
            id=CHARACTER_ID,
            object="character",
            created_at=CHARACTER_CREATED_AT,
            name="Wren" if call.name is None else call.name,
        )


def recorded_call(
    arguments: SharedHandlerArguments,
    function: Literal["create", "get"],
    primary: str,
    media: str | None = None,
) -> RecordedCall:
    return RecordedCall(
        function=function,
        provider=arguments["custom_llm_provider"],
        timeout=0.0 if arguments["timeout"] is None else float(arguments["timeout"]),
        asynchronous=arguments["_is_async"],
        api_base=arguments["litellm_params"].api_base,
        api_key=arguments["litellm_params"].api_key,
        extra_headers=arguments["extra_headers"],
        name=primary if function == "create" else None,
        video=media,
        character_id=primary if function == "get" else None,
    )


def call_kwargs(call: SharedInput) -> dict[str, Arg]:
    kwargs: dict[str, Arg] = {"litellm_call_id": call["litellm_call_id"]}
    if "custom_llm_provider" in call:
        kwargs["custom_llm_provider"] = call["custom_llm_provider"]
    if "extra_headers" in call:
        kwargs["extra_headers"] = call["extra_headers"]
    if "extra" in call:
        kwargs.update(call["extra"])
    if "timeout" in call:
        kwargs["timeout"] = call["timeout"]
    return kwargs


async def resolved(created: Created) -> CharacterObject:
    if inspect.isawaitable(created):
        return await created
    return created


def answer(input_value: Input, logging_obj: Logging) -> CharacterObject:
    kwargs = call_kwargs(input_value)
    if input_value["function"] == "create":
        creator = (
            API.avideo_create_character
            if input_value["asynchronous"]
            else API.video_create_character
        )
        created = creator(
            input_value["name"],
            Path(input_value["video"]),
            litellm_logging_obj=logging_obj,
            **kwargs,
        )
    else:
        getter = (
            API.avideo_get_character
            if input_value["asynchronous"]
            else API.video_get_character
        )
        created = getter(
            input_value["character_id"], litellm_logging_obj=logging_obj, **kwargs
        )
    return asyncio.run(resolved(created))


def run(input_value: Input) -> JsonValue:
    recorder = Recorder()
    logging_obj = Logging(
        model="character",
        messages=None,
        stream=False,
        call_type="fixture_call_type",
        start_time=START,
        litellm_call_id="call-1",
        function_id="fn-1",
    )
    handlers = VIDEOS.base_llm_http_handler
    create_handler = handlers.video_create_character_handler
    get_handler = handlers.video_get_character_handler
    handlers.video_create_character_handler = recorder.video_create_character_handler
    handlers.video_get_character_handler = recorder.video_get_character_handler
    try:
        result = answer(input_value, logging_obj)
    finally:
        handlers.video_create_character_handler = create_handler
        handlers.video_get_character_handler = get_handler
    logging = recorder.logging
    if logging is None:
        raise ValueError("the handler never answered")
    output = Output(
        result=JSON.validate_json(result.model_dump_json()),
        calls=recorder.calls,
        logging=logging,
    )
    return JSON.validate_json(output.model_dump_json())
