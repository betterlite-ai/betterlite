import asyncio
from typing import Final, Literal, Protocol, TypeAlias, cast

import litellm
import litellm.images.main as images_main
import openai
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.image_edit.transformation import BaseImageEditConfig
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.llms.custom_llm import CustomLLM
from litellm.types.llms.custom_llm import CustomLLMItem
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import ImageResponse, LlmProviders
from litellm.utils import ProviderConfigManager, custom_llm_setup
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter
from typing_extensions import override

from image_edit_http import FixtureConfig
from image_variations import Success, api_failure, datum
from openai_chat import Failure

Argument: TypeAlias = JsonValue | bytes | list[bytes] | HTTPHandler | AsyncHTTPHandler
Unused: TypeAlias = Logging | ImageResponse | GenericLiteLLMParams | BaseImageEditConfig | Argument


class ImageEditEntry(Protocol):
    def image_edit(self, /, **kwargs: Argument) -> ImageResponse: ...

    async def aimage_edit(self, /, **kwargs: Argument) -> ImageResponse: ...


ENTRY = cast("ImageEditEntry", images_main)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L1674
PARAMS: Final = TypeAdapter(dict[str, JsonValue])
FIXTURE: Final = FixtureConfig(multipart=False)
SELECT_CONFIG: Final = ProviderConfigManager.get_provider_image_edit_config
BEDROCK_EDIT: Final = images_main.bedrock_image_edit.image_edit
BFL_EDIT: Final = images_main.bfl_image_edit.image_edit
EDIT_HANDLER: Final = images_main.base_llm_http_handler.image_edit_handler


class CustomProvider(BaseModel):
    model_config = ConfigDict(extra="forbid")

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
    client: Literal["sync", "async"] | None = None
    custom_providers: list[CustomProvider] = Field(default_factory=list)


class Call(BaseModel):
    model_config = ConfigDict(extra="forbid")

    runs: list[Run]


class Sent(BaseModel):
    provider: str
    model: str
    prompt: str | None
    images: list[str]
    optional_params: dict[str, JsonValue]
    extra_headers: dict[str, JsonValue] | None
    extra_body: dict[str, JsonValue] | None
    timeout: float | None
    asynchronous: bool


class CustomRecord(BaseModel):
    handler: str
    model: str
    images: list[str]
    prompt: str | None
    optional_params: dict[str, JsonValue]
    timeout: float | None
    client: Literal["sync", "async"] | None
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


class Recorder:
    def __init__(self) -> None:
        self.sent: Sent | None = None

    def bedrock(
        self,
        *,
        model: str,
        image: list[bytes],
        prompt: str | None,
        optional_params: dict[str, JsonValue],
        timeout: float | None,
        aimage_edit: bool,
        extra_headers: dict[str, JsonValue] | None,
        **_unused: Unused,
    ) -> ImageResponse:
        self.sent = Sent(
            provider="bedrock",
            model=model,
            prompt=prompt,
            images=[part.decode() for part in image],
            optional_params=PARAMS.validate_python(optional_params),
            extra_headers=extra_headers,
            extra_body=None,
            timeout=timeout,
            asynchronous=aimage_edit,
        )
        return ImageResponse(created=1, data=[])

    def black_forest_labs(
        self,
        *,
        model: str,
        image: list[bytes],
        prompt: str | None,
        # ast-grep-ignore: no-long-names
        image_edit_optional_request_params: dict[str, JsonValue],
        timeout: float | None,
        aimage_edit: bool,
        extra_headers: dict[str, JsonValue] | None,
        **_unused: Unused,
    ) -> ImageResponse:
        self.sent = Sent(
            provider="black_forest_labs",
            model=model,
            prompt=prompt,
            images=[part.decode() for part in image],
            # ast-grep-ignore: no-long-names
            optional_params=PARAMS.validate_python(image_edit_optional_request_params),
            extra_headers=extra_headers,
            extra_body=None,
            timeout=timeout,
            asynchronous=aimage_edit,
        )
        return ImageResponse(created=1, data=[])

    def handler(
        self,
        *,
        model: str,
        image: list[bytes],
        prompt: str | None,
        # ast-grep-ignore: no-long-names
        image_edit_optional_request_params: dict[str, JsonValue],
        custom_llm_provider: str,
        extra_headers: dict[str, JsonValue] | None,
        extra_body: dict[str, JsonValue] | None,
        timeout: float | None,
        _is_async: bool,
        **_unused: Unused,
    ) -> ImageResponse:
        self.sent = Sent(
            provider=custom_llm_provider,
            model=model,
            prompt=prompt,
            images=[part.decode() for part in image],
            # ast-grep-ignore: no-long-names
            optional_params=PARAMS.validate_python(image_edit_optional_request_params),
            extra_headers=extra_headers,
            extra_body=extra_body,
            timeout=timeout,
            asynchronous=_is_async,
        )
        return ImageResponse(created=1, data=[])


def recorded_config(model: str, provider: LlmProviders) -> BaseImageEditConfig | None:
    if SELECT_CONFIG(model=model, provider=provider) is None:
        return None
    return FIXTURE


def client_kind(client: HTTPHandler | AsyncHTTPHandler | None) -> Literal["sync", "async"] | None:
    if client is None:
        return None
    match client:
        case AsyncHTTPHandler():
            return "async"
        case HTTPHandler():
            return "sync"


class Handler(CustomLLM):
    def __init__(self, name: str, kwargs: list[str]) -> None:
        super().__init__()
        self.name = name
        self.kwargs = kwargs
        self.received: CustomRecord | None = None

    def record(
        self,
        values: tuple[str, list[bytes], str | None, dict[str, Argument | Logging], float | None],
        client: HTTPHandler | AsyncHTTPHandler | None,
        asynchronous: bool,
    ) -> ImageResponse:
        model, image, prompt, optional_params, timeout = values
        self.received = CustomRecord(
            handler=self.name,
            model=model,
            images=[part.decode() for part in image],
            prompt=prompt,
            optional_params=PARAMS.validate_python(
                {name: optional_params[name] for name in self.kwargs if name in optional_params}
            ),
            timeout=timeout,
            client=client_kind(client),
            asynchronous=asynchronous,
        )
        return ImageResponse(created=1, data=[])

    @override
    def image_edit(
        self,
        model: str,
        image: list[bytes],
        prompt: str | None,
        model_response: ImageResponse,
        api_key: str | None,
        api_base: str | None,
        optional_params: dict[str, Argument | Logging],
        logging_obj: Logging,
        timeout: float | None = None,
        client: HTTPHandler | None = None,
    ) -> ImageResponse:
        return self.record((model, image, prompt, optional_params, timeout), client, asynchronous=False)

    @override
    async def aimage_edit(
        self,
        model: str,
        image: list[bytes],
        prompt: str | None,
        model_response: ImageResponse,
        api_key: str | None,
        api_base: str | None,
        optional_params: dict[str, Argument | Logging],
        logging_obj: Logging,
        timeout: float | None = None,
        client: AsyncHTTPHandler | None = None,
    ) -> ImageResponse:
        return self.record((model, image, prompt, optional_params, timeout), client, asynchronous=True)


def register(run: Run) -> list[Handler]:
    handlers: list[Handler] = []
    entries: list[CustomLLMItem] = []
    for provider in run.custom_providers:
        if not provider.handlers:
            litellm.provider_list.append(provider.name)
            litellm._custom_providers.append(provider.name)  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L784
        for name in provider.handlers:
            handler = Handler(name, list(run.kwargs))
            handlers.append(handler)
            entries.append({"provider": provider.name, "custom_handler": handler})
    litellm.custom_provider_map = entries
    custom_llm_setup()
    return handlers


def arguments(run: Run, api_base: str) -> dict[str, Argument]:
    values: dict[str, Argument] = dict(run.kwargs)
    if not run.custom_providers:
        values["api_base"] = api_base
    match run.client:
        case "sync":
            values["client"] = HTTPHandler()
        case "async":
            values["client"] = AsyncHTTPHandler()
        case None:
            pass
    values["image"] = None
    if run.image is not None:
        match run.image:
            case str() as content:
                values["image"] = content.encode()
            case list() as parts:
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


def recorded_call(run: Run, api_base: str, recorder: Recorder) -> ImageResponse:
    ProviderConfigManager.get_provider_image_edit_config = recorded_config  # type: ignore[method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L833-L836
    images_main.bedrock_image_edit.image_edit = recorder.bedrock  # type: ignore[assignment, method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L888
    images_main.bfl_image_edit.image_edit = recorder.black_forest_labs  # type: ignore[assignment, method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L924
    images_main.base_llm_http_handler.image_edit_handler = recorder.handler  # type: ignore[assignment, method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L937
    try:
        return call_entry(run, api_base)
    finally:
        ProviderConfigManager.get_provider_image_edit_config = SELECT_CONFIG  # type: ignore[method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L833-L836
        images_main.bedrock_image_edit.image_edit = BEDROCK_EDIT  # type: ignore[method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L888
        images_main.bfl_image_edit.image_edit = BFL_EDIT  # type: ignore[method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L924
        images_main.base_llm_http_handler.image_edit_handler = EDIT_HANDLER  # type: ignore[method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/main.py#L937


def run_one(run: Run, api_base: str) -> RunOutcome:
    handlers = register(run)
    recorder = Recorder()
    try:
        if run.recorded:
            response = recorded_call(run, api_base, recorder)
        else:
            response = call_entry(run, api_base)
    except openai.APIError as error:
        return RunOutcome(failure=api_failure(error))
    data = response.data if response.data is not None else []
    outcome = RunOutcome(success=Success(created=response.created, data=[datum(image) for image in data]))
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
        return Envelope(image_edit_call=ImageEditCallValue(value=Outcome(runs=runs))).model_dump_json(
            by_alias=True, exclude_unset=True
        )
