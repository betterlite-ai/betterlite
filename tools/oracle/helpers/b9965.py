import asyncio
from collections.abc import Callable, Coroutine, Mapping
from typing import Literal, Protocol, TypeAlias, TypedDict, Unpack

import litellm
from litellm.exceptions import ImageFetchError
from litellm.litellm_core_utils.prompt_templates import factory, image_handling
from litellm.litellm_core_utils.prompt_templates.image_handling import (
    RemoteMedia,
    async_inline_remote_media,
    inline_remote_media,
)
from litellm.llms.gemini.chat import transformation as studio_transformation
from litellm.llms.vertex_ai.gemini import transformation
from litellm.llms.vertex_ai.gemini.transformation import (
    _ai_studio_inlines,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/transformation.py#L1318-L1319
    _transform_request_body,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/transformation.py#L1149-L1160
    _vertex_inlines,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/transformation.py#L1322-L1327
)
from openai import BadRequestError
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

JsonObject: TypeAlias = dict[str, Json]
Provider: TypeAlias = Literal["vertex_ai", "vertex_ai_beta", "gemini"]
Mode: TypeAlias = Literal["sequential", "concurrent"]
Stage: TypeAlias = Literal["prepare", "convert"]
Predicate: TypeAlias = Callable[[RemoteMedia], bool]
SyncInline: TypeAlias = Callable[[list[JsonObject], Predicate], list[JsonObject]]
AsyncInline: TypeAlias = Callable[
    [list[JsonObject], Predicate], Coroutine[None, None, list[JsonObject]]
]

JSON_OBJECT = TypeAdapter(dict[str, Json])
STUDIO_INLINES: Predicate = _ai_studio_inlines  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/transformation.py#L1318-L1319
VERTEX_INLINES: Predicate = _vertex_inlines  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/transformation.py#L1322-L1327

litellm.suppress_debug_info = True


class TransformArgs(TypedDict):
    messages: list[JsonObject]
    model: str
    optional_params: JsonObject
    custom_llm_provider: Provider
    litellm_params: JsonObject
    cached_content: None


class Transform(Protocol):
    def __call__(self, **kwargs: Unpack[TransformArgs]) -> Mapping[str, Json]: ...


# LiteLLM's transform takes bare dicts and its RequestBody TypedDicts carry Any
TRANSFORM: Transform = _transform_request_body  # type: ignore[assignment]  # pyright: ignore[reportAssignmentType, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/transformation.py#L1149-L1160
INLINE: SyncInline = inline_remote_media  # type: ignore[assignment]  # pyright: ignore[reportAssignmentType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/prompt_templates/image_handling.py#L314-L322
ASYNC_INLINE: AsyncInline = async_inline_remote_media  # type: ignore[assignment]  # pyright: ignore[reportAssignmentType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/prompt_templates/image_handling.py#L325-L334


class Case(TypedDict):
    model: str
    provider: Provider
    mode: Mode
    messages: list[JsonObject]
    data_urls: dict[str, str]
    optional_params: JsonObject
    litellm_params: JsonObject


class MappedConvert:
    def __init__(self, data_urls: dict[str, str], attempted: list[Json]) -> None:
        self.data_urls = data_urls
        self.attempted = attempted

    def __call__(self, url: str) -> str:
        self.attempted.append(url)
        if url not in self.data_urls:
            message = f"Error: Unable to fetch image from URL after 3 attempts. url={url}"
            raise ImageFetchError(message)
        return self.data_urls[url]


class AsyncMappedConvert:
    def __init__(self, data_urls: dict[str, str], attempted: list[Json]) -> None:
        self.data_urls = data_urls
        self.attempted = attempted

    async def __call__(self, url: str) -> str:
        self.attempted.append(url)
        if url not in self.data_urls:
            message = f"Error: Unable to fetch image from URL after 3 attempts. url={url}"
            raise ImageFetchError(message)
        return self.data_urls[url]


def _no_gcs_metadata(
    image_url: str,
    vertex_project: str | None = None,
    vertex_credentials: Json = None,
) -> str | None:
    message = f"no GCS content type for {image_url} ({vertex_project}, {vertex_credentials})"
    raise RuntimeError(message)


def _install_fetches(data_urls: dict[str, str]) -> list[Json]:
    attempted: list[Json] = []
    mapped = MappedConvert(data_urls, attempted)
    image_handling.convert_url_to_base64 = mapped
    image_handling.async_convert_url_to_base64 = AsyncMappedConvert(data_urls, attempted)
    factory.convert_url_to_base64 = mapped  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/prompt_templates/factory.py#L59
    studio_transformation.convert_url_to_base64 = mapped  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/chat/transformation.py#L9
    transformation._get_gcs_object_content_type = _no_gcs_metadata  # type: ignore[assignment]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAssignmentType, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/transformation.py#L329-L333
    return attempted


def _predicate(provider: Provider) -> Predicate:
    if provider == "gemini":
        return STUDIO_INLINES
    return VERTEX_INLINES


def _inline(input_value: Case) -> list[JsonObject]:
    predicate = _predicate(input_value["provider"])
    if input_value["mode"] == "sequential":
        return INLINE(input_value["messages"], predicate)
    return asyncio.run(ASYNC_INLINE(input_value["messages"], predicate))


# compare_helper drops the error class and message, so a failure travels in the compared value
def _failure(stage: Stage, attempted: list[Json], status: int) -> JsonObject:
    failure: JsonObject = {"status": status, "type": None, "param": None, "code": None}
    return {"failure": failure, "stage": stage, "fetched": attempted}


def run(input_value: Case) -> JsonObject:
    attempted = _install_fetches(input_value["data_urls"])
    try:
        messages = _inline(input_value)
    except BadRequestError as error:
        return _failure("prepare", attempted, error.status_code)
    optional_params = dict(input_value["optional_params"])
    litellm_params = dict(input_value["litellm_params"])
    try:
        request = TRANSFORM(
            messages=messages,
            model=input_value["model"],
            optional_params=optional_params,
            custom_llm_provider=input_value["provider"],
            litellm_params=litellm_params,
            cached_content=None,
        )
    except BadRequestError as error:
        return _failure("convert", attempted, error.status_code)
    return {
        "body": JSON_OBJECT.validate_python(request),
        "litellm_params": JSON_OBJECT.validate_python(litellm_params),
        "fetched": attempted,
    }
