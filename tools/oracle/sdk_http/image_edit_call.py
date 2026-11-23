import asyncio
from contextlib import nullcontext
from functools import partial
from types import TracebackType
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, Unpack, cast

import litellm
import litellm.images.main as images_main
import openai
from httpx import Timeout
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.image_edit.transformation import BaseImageEditConfig
from litellm.llms.black_forest_labs.image_edit.handler import bfl_image_edit
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.llms.custom_llm import CustomLLM
from litellm.main import base_llm_http_handler, bedrock_image_edit
from litellm.types.llms.custom_llm import CustomLLMItem
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import ImageResponse, LlmProviders
from litellm.utils import ProviderConfigManager, custom_llm_setup
from pydantic import AliasChoices, BaseModel, ConfigDict, Field, JsonValue
from typing_extensions import override

from image_edit_http import FixtureConfig
from image_variations import Success, api_failure, datum
from openai_chat import Failure

Client: TypeAlias = HTTPHandler | AsyncHTTPHandler
Argument: TypeAlias = JsonValue | bytes | list[bytes] | Client
Seen: TypeAlias = Logging | ImageResponse | GenericLiteLLMParams | BaseImageEditConfig | Argument
ClientKind: TypeAlias = Literal["sync", "async"]


class ImageEditEntry(Protocol):
    def image_edit(self, /, **kwargs: Argument) -> ImageResponse: ...

    async def aimage_edit(self, /, **kwargs: Argument) -> ImageResponse: ...


ENTRY = cast("ImageEditEntry", images_main)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L1674
FIXTURE: Final = FixtureConfig(multipart=False)
SELECT_CONFIG: Final = ProviderConfigManager.get_provider_image_edit_config
BEDROCK_EDIT: Final = bedrock_image_edit.image_edit  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L888
BFL_EDIT: Final = bfl_image_edit.image_edit  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L924
EDIT_HANDLER: Final = base_llm_http_handler.image_edit_handler  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L937


class CustomProvider(BaseModel):
    name: str
    handlers: list[str] = Field(default_factory=list)


class Run(BaseModel):
    model_config = ConfigDict(extra="forbid")

    asynchronous: bool = False
    recorded: bool = False
    image: str | list[str] | None = None
    prompt: str | None = None
    model: str | None = None
    mask: str | None = None
    n: int | None = None
    quality: Literal["high", "medium", "low", "standard", "auto"] | None = None
    response_format: Literal["url", "b64_json"] | None = None
    size: str | None = None
    user: str | None = None
    extra_headers: dict[str, JsonValue] | None = None
    extra_query: dict[str, JsonValue] | None = None
    extra_body: dict[str, JsonValue] | None = None
    timeout: float | None = None
    custom_llm_provider: str | None = None
    kwargs: dict[str, JsonValue] = Field(default_factory=dict)
    client: ClientKind | None = None
    custom_providers: list[CustomProvider] = Field(default_factory=list)


class Call(BaseModel):
    runs: list[Run]


# bedrock_image_edit, bfl_image_edit and the generic handler name the same arguments differently
class Sent(BaseModel):
    model_config = ConfigDict(validate_by_name=True)

    provider: str = Field(validation_alias="custom_llm_provider")
    model: str
    prompt: str | None
    images: list[str] = Field(validation_alias="image")
    optional_params: dict[str, JsonValue] = Field(
        default_factory=dict,
        validation_alias=AliasChoices("optional_params", "image_edit_optional_request_params"),
    )
    extra_headers: dict[str, JsonValue] | None
    extra_body: dict[str, JsonValue] | None = None
    timeout: float | None
    asynchronous: bool = Field(
        default=False, validation_alias=AliasChoices("aimage_edit", "_is_async")
    )


class CustomRecord(BaseModel):
    handler: str
    model: str
    images: list[str]
    prompt: str | None
    optional_params: dict[str, JsonValue]
    timeout: float | None
    client: ClientKind | None
    asynchronous: bool


class RunOutcome(BaseModel):
    success: Success | None = None
    failure: Failure | None = None
    sent: Sent | None = None
    custom: CustomRecord | None = None


class Outcome(BaseModel):
    runs: list[RunOutcome]


class ImageEditCallValue(BaseModel):
    value: Outcome


class Envelope(BaseModel):
    image_edit_call: ImageEditCallValue


# the keyword arguments image_edit hands a custom handler
class CustomArguments(TypedDict):
    model: str
    image: list[bytes]
    prompt: str | None
    model_response: ImageResponse
    api_key: str | None
    api_base: str | None
    optional_params: dict[str, Argument | Logging]
    logging_obj: Logging
    timeout: float | Timeout | None
    client: Client | None


class Recorder:
    def __init__(self) -> None:
        self.sent: Sent | None = None

    def keep(self, provider: str, **received: Seen) -> ImageResponse:
        values = {"custom_llm_provider": provider, "extra_body": None, **received}
        self.sent = Sent.model_validate(values)
        return ImageResponse(created=1, data=[])

    # each configured send records for one call, and LiteLLM's own sends come back after it
    def __enter__(self) -> None:
        ProviderConfigManager.get_provider_image_edit_config = recorded_config  # type: ignore[method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L833-L836
        bedrock_image_edit.image_edit = partial(self.keep, "bedrock")  # type: ignore[method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L888
        bfl_image_edit.image_edit = partial(self.keep, "black_forest_labs")  # type: ignore[method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L924
        base_llm_http_handler.image_edit_handler = partial(self.keep, "")  # type: ignore[method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L937

    def __exit__(
        self,
        _exc_type: type[BaseException] | None,
        _exc: BaseException | None,
        _traceback: TracebackType | None,
    ) -> None:
        ProviderConfigManager.get_provider_image_edit_config = SELECT_CONFIG  # type: ignore[method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L833-L836
        bedrock_image_edit.image_edit = BEDROCK_EDIT  # type: ignore[method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L888
        bfl_image_edit.image_edit = BFL_EDIT  # type: ignore[method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L924
        base_llm_http_handler.image_edit_handler = EDIT_HANDLER  # type: ignore[method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L937


def recorded_config(model: str, provider: LlmProviders) -> BaseImageEditConfig | None:
    if SELECT_CONFIG(model=model, provider=provider) is None:
        return None
    return FIXTURE


class Handler(CustomLLM):
    def __init__(self, name: str, kwargs: list[str]) -> None:
        super().__init__()
        self.name = name
        self.kwargs = kwargs
        self.received: CustomRecord | None = None

    # image_edit hands over a client only of the kind the entry expects, None otherwise
    def record(self, call: CustomArguments, kind: ClientKind) -> ImageResponse:
        params = call["optional_params"]
        forwarded = {name: params[name] for name in self.kwargs if name in params}
        values = {
            "handler": self.name,
            "model": call["model"],
            "images": [part.decode() for part in call["image"]],
            "prompt": call["prompt"],
            "optional_params": forwarded,
            "timeout": call["timeout"],
            "client": None if call["client"] is None else kind,
            "asynchronous": kind == "async",
        }
        self.received = CustomRecord.model_validate(values)
        return ImageResponse(created=1, data=[])

    @override
    def image_edit(self, **call: Unpack[CustomArguments]) -> ImageResponse:  # type: ignore[override]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L813-L824
        return self.record(call, "sync")

    @override
    async def aimage_edit(self, **call: Unpack[CustomArguments]) -> ImageResponse:  # type: ignore[override]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L798-L809
        return self.record(call, "async")


def register(run: Run) -> list[Handler]:
    handlers: list[Handler] = []
    entries: list[CustomLLMItem] = []
    for provider in run.custom_providers:
        if not provider.handlers:
            litellm.provider_list.append(provider.name)
            litellm._custom_providers.append(provider.name)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L784
        for name in provider.handlers:
            handler = Handler(name, list(run.kwargs))
            handlers.append(handler)
            entries.append({"provider": provider.name, "custom_handler": handler})
    litellm.custom_provider_map = entries
    custom_llm_setup()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L572
    return handlers


def arguments(run: Run, api_base: str) -> dict[str, Argument]:
    values: dict[str, Argument] = {**run.kwargs}
    if not run.custom_providers:
        values["api_base"] = api_base
    if run.client is not None:
        values["client"] = HTTPHandler() if run.client == "sync" else AsyncHTTPHandler()
    values["image"] = None
    if run.image is not None:
        match run.image:
            case str() as content:
                values["image"] = content.encode()
            case [*parts]:
                values["image"] = [part.encode() for part in parts]
    values.update(
        prompt=run.prompt,
        model=run.model,
        mask=None if run.mask is None else run.mask.encode(),
        n=run.n,
        quality=run.quality,
        response_format=run.response_format,
        size=run.size,
        user=run.user,
        extra_headers=run.extra_headers,
        extra_query=run.extra_query,
        extra_body=run.extra_body,
        timeout=run.timeout,
        custom_llm_provider=run.custom_llm_provider,
    )
    return values


def call_entry(run: Run, api_base: str) -> ImageResponse:
    values = arguments(run, api_base)
    if run.asynchronous:
        return asyncio.run(ENTRY.aimage_edit(**values))
    return ENTRY.image_edit(**values)


def run_one(run: Run, api_base: str) -> RunOutcome:
    handlers = register(run)
    recorder = Recorder()
    try:
        with recorder if run.recorded else nullcontext():
            response = call_entry(run, api_base)
    except openai.APIError as error:
        return RunOutcome(failure=api_failure(error))
    data = response.data if response.data is not None else []
    success = Success(created=response.created, data=[datum(image) for image in data])
    outcome = RunOutcome(success=success)
    if recorder.sent is not None:
        outcome.sent = recorder.sent
    for handler in handlers:
        if handler.received is not None:
            outcome.custom = handler.received
    return outcome


class ImageEditCallRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    image_edit_call: Call

    def serialize(self, api_base: str) -> str:
        runs = [run_one(run, api_base) for run in self.image_edit_call.runs]
        envelope = Envelope(image_edit_call=ImageEditCallValue(value=Outcome(runs=runs)))
        return envelope.model_dump_json(by_alias=True, exclude_unset=True)
