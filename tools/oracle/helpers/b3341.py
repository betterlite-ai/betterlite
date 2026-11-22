import asyncio
import datetime as dt
import inspect
from collections.abc import Coroutine
from typing import Final, Literal, NotRequired, Protocol, TypedDict
from unittest.mock import patch

import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.videos.transformation import BaseVideoConfig
from litellm.types.router import GenericLiteLLMParams
from litellm.types.videos.main import CharacterObject
from litellm.videos import main as videos_main
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

litellm.suppress_debug_info = True

JSON: Final = TypeAdapter(JsonValue)
START: Final = dt.datetime(2024, 1, 1, tzinfo=dt.UTC)


class Input(TypedDict):
    function: Literal["create", "get"]
    asynchronous: bool
    litellm_call_id: str
    name: NotRequired[str]
    video: NotRequired[str]
    character_id: NotRequired[str]
    timeout: NotRequired[float | None]
    custom_llm_provider: NotRequired[str | None]
    extra_headers: NotRequired[dict[str, str] | None]
    extra: NotRequired[dict[str, JsonValue]]


Created = CharacterObject | Coroutine[object, object, CharacterObject]


class CharacterApi(Protocol):
    # LiteLLM leaves **kwargs unannotated, so the view pins the positional arguments and their types
    def video_create_character(
        self, name: str, video: str, *, litellm_logging_obj: Logging, **kwargs: object
    ) -> Created: ...
    def avideo_create_character(
        self, name: str, video: str, *, litellm_logging_obj: Logging, **kwargs: object
    ) -> Created: ...
    def video_get_character(
        self, character_id: str, *, litellm_logging_obj: Logging, **kwargs: object
    ) -> Created: ...
    def avideo_get_character(
        self, character_id: str, *, litellm_logging_obj: Logging, **kwargs: object
    ) -> Created: ...


CHARACTER_API: CharacterApi = videos_main


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
    def __init__(self, video: str | None) -> None:
        self.video = video
        self.calls: list[RecordedCall] = []
        self.logging: RecordedLogging | None = None

    def create_handler(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/llm_http_handler.py#L7736-L7749
        self,
        *,
        name: str,
        video: object,
        video_provider_config: BaseVideoConfig,
        custom_llm_provider: str,
        litellm_params: GenericLiteLLMParams,
        logging_obj: Logging,
        extra_headers: dict[str, str] | None = None,
        timeout: float | None = None,
        _is_async: bool = False,
        client: object | None = None,
        api_key: str | None = None,
    ) -> CharacterObject:
        del video, video_provider_config, client, api_key
        self.record(
            RecordedCall(
                function="create",
                provider=custom_llm_provider,
                timeout=recorded_timeout(timeout),
                asynchronous=_is_async,
                api_base=litellm_params.api_base,
                api_key=litellm_params.api_key,
                extra_headers=extra_headers,
                name=name,
                video=self.video,
                character_id=None,
            ),
            logging_obj,
        )
        return CharacterObject(
            id="char-1", object="character", created_at=1700000000, name=name
        )

    def get_handler(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/llm_http_handler.py#L7892-L7905
        self,
        *,
        character_id: str,
        video_provider_config: BaseVideoConfig,
        custom_llm_provider: str,
        litellm_params: GenericLiteLLMParams,
        logging_obj: Logging,
        extra_headers: dict[str, str] | None = None,
        timeout: float | None = None,
        _is_async: bool = False,
        client: object | None = None,
        api_key: str | None = None,
    ) -> CharacterObject:
        del video_provider_config, client, api_key
        self.record(
            RecordedCall(
                function="get",
                provider=custom_llm_provider,
                timeout=recorded_timeout(timeout),
                asynchronous=_is_async,
                api_base=litellm_params.api_base,
                api_key=litellm_params.api_key,
                extra_headers=extra_headers,
                name=None,
                video=None,
                character_id=character_id,
            ),
            logging_obj,
        )
        return CharacterObject(
            id="char-1", object="character", created_at=1700000000, name="Wren"
        )

    def record(self, call: RecordedCall, logging_obj: Logging) -> None:
        self.calls.append(call)
        self.logging = RecordedLogging.model_validate(logging_obj)


def recorded_timeout(timeout: float | None) -> float:
    return 0.0 if timeout is None else float(timeout)


def call_kwargs(call: Input, logging_obj: Logging) -> dict[str, object]:
    kwargs: dict[str, object] = {
        "litellm_call_id": call["litellm_call_id"],
        "custom_llm_provider": call.get("custom_llm_provider"),
        "extra_headers": call.get("extra_headers"),
        **call.get("extra", {}),
    }
    if "timeout" in call:
        kwargs["timeout"] = call["timeout"]
    return kwargs


async def resolved(created: Created) -> CharacterObject:
    if inspect.isawaitable(created):
        return await created
    return created


def answer(call: Input, recorder: Recorder, logging_obj: Logging) -> CharacterObject:
    kwargs = call_kwargs(call, logging_obj)
    if call["function"] == "create":
        creator = (
            CHARACTER_API.avideo_create_character
            if call["asynchronous"]
            else CHARACTER_API.video_create_character
        )
        return asyncio.run(resolved(creator(call["name"], call["video"], **kwargs)))
    getter = (
        CHARACTER_API.avideo_get_character
        if call["asynchronous"]
        else CHARACTER_API.video_get_character
    )
    return asyncio.run(resolved(getter(call["character_id"], **kwargs)))


def run(input_value: Input) -> JsonValue:
    recorder = Recorder(input_value.get("video"))
    logging_obj = Logging(
        model="character",
        messages=None,
        stream=False,
        call_type="fixture_call_type",
        start_time=START,
        litellm_call_id="call-1",
        function_id="fn-1",
    )
    with (
        patch.object(
            videos_main.base_llm_http_handler,
            "video_create_character_handler",
            recorder.create_handler,
        ),
        patch.object(
            videos_main.base_llm_http_handler,
            "video_get_character_handler",
            recorder.get_handler,
        ),
    ):
        result = answer(input_value, recorder, logging_obj)
    logging = recorder.logging
    if logging is None:
        raise ValueError("the handler never answered")
    output = Output(
        result=JSON.validate_json(result.model_dump_json()),
        calls=recorder.calls,
        logging=logging,
    )
    return JSON.validate_json(output.model_dump_json())
