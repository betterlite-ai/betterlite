from collections.abc import Callable
from datetime import UTC, datetime
from typing import Final, Protocol, TypeAlias

import httpx
import litellm
import openai
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.image_variations.transformation import BaseImageVariationConfig
from litellm.llms.openai.common_utils import OpenAIError
from litellm.llms.openai.image_variations import handler as variation_handler
from litellm.llms.openai.image_variations.handler import OpenAIImageVariationsHandler
from litellm.types.utils import FileTypes, ImageResponse, LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import BaseModel
from pydantic import JsonValue as Json
from typing_extensions import override

from .config import variation_config
from .probe import DOCUMENT, NOW, Probe, file_object, text_of
from .transform import empty_answer

litellm.suppress_debug_info = True

# LiteLLM's class carries Any in its constructor, so the catch names it through a typed alias
PROVIDER_ERROR: Final[type[Exception]] = OpenAIError


Variation: TypeAlias = Callable[
    [
        ImageResponse,
        str,
        str,
        str | None,
        FileTypes,
        float | None,
        str,
        Logging,
        dict[str, Json],
        dict[str, Json],
        None,
        None,
        openai.OpenAI | None,
        str | None,
        dict[str, str] | None,
    ],
    ImageResponse,
]


class VariationHandler(Protocol):
    @property
    def image_variations(self) -> Variation: ...


class AbsentProviderConfigManager(ProviderConfigManager):
    @staticmethod
    @override
    def get_provider_image_variation_config(  # ast-grep-ignore: no-long-names
        model: str, provider: LlmProviders
    ) -> BaseImageVariationConfig | None:
        # the fake registry has no OpenAI image variation config
        if provider == LlmProviders.OPENAI:
            return None
        return ProviderConfigManager.get_provider_image_variation_config(model, provider)


CONFIG_MANAGER: Final = "ProviderConfigManager"


class ErrorRecord(BaseModel):
    status_code: int
    message: str
    headers: dict[str, str] | None = None


class Upstream:
    def __init__(self, probe: Probe) -> None:
        self.probe = probe
        self.seen: list[httpx.Request] = []

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.seen.append(request)
        match self.probe.failure:
            case "timeout":
                raise httpx.ReadTimeout("read timed out", request=request)
            case "failed":
                raise httpx.ReadError("the transport failed", request=request)
            case None:
                body = self.probe.reply.body.encode()
                if self.probe.reply.body_bytes is not None:
                    body = bytes(self.probe.reply.body_bytes)
                return httpx.Response(
                    self.probe.reply.status,
                    headers=self.probe.reply.headers,
                    content=body,
                )


def supplied_client(probe: Probe, transport: httpx.MockTransport) -> openai.OpenAI | None:
    if probe.client != "sync":
        return None
    return openai.OpenAI(
        api_key="sk-supplied",
        base_url="https://supplied.test/v1",
        organization="org-supplied",
        timeout=45.0,
        max_retries=1,
        http_client=httpx.Client(transport=transport),
    )


def logging_object() -> Logging:
    return Logging(
        model="image-variation",
        messages=None,
        stream=False,
        call_type="image_generation",
        start_time=datetime(2023, 11, 14, tzinfo=UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
    )


def timeout_of(probe: Probe) -> float | None:
    if probe.timeout is None:
        return None
    return float(probe.timeout)


def header_value(headers: httpx.Headers, name: str) -> str | None:
    values = headers.get_list(name)
    if not values:
        return None
    return values[0]


def multipart_param(header: str, name: str) -> str | None:
    for piece in header.split(";"):
        key, _, value = piece.strip().partition("=")
        if key.lower() == name:
            return value.strip('"')
    return None


def boundary_of(content_type: str) -> str:
    boundary = multipart_param(content_type, "boundary")
    if boundary is None:
        raise ValueError("a multipart request names its boundary")
    return boundary


def part_of(chunk: bytes) -> dict[str, Json]:
    head, _, body = chunk.partition(b"\r\n\r\n")
    fields: dict[str, str] = {}
    for line in head.split(b"\r\n"):
        name, _, value = line.decode("utf-8").partition(":")
        fields[name.strip().lower()] = value.strip()
    disposition = fields.get("content-disposition")
    if disposition is None:
        raise ValueError("a multipart part names its disposition")
    return {
        "name": multipart_param(disposition, "name"),
        "filename": multipart_param(disposition, "filename"),
        "content_type": fields.get("content-type"),
        "data": body.removesuffix(b"\r\n").decode("utf-8"),
    }


def parts_view(request: httpx.Request) -> list[Json]:
    content_type = header_value(request.headers, "content-type")
    if content_type is None:
        raise ValueError("a multipart request carries a content type")
    delimiter = b"--" + boundary_of(content_type).encode()
    parts: list[Json] = []
    for chunk in request.read().split(delimiter)[1:]:
        if chunk.startswith(b"--"):
            break
        parts.append(part_of(chunk.removeprefix(b"\r\n")))
    return parts


def calls_view(seen: list[httpx.Request]) -> list[Json]:
    calls: list[Json] = []
    for request in seen:
        call: dict[str, Json] = {
            "url": str(request.url),
            "authorization": header_value(request.headers, "authorization"),
            "organization": header_value(request.headers, "openai-organization"),
            "parts": parts_view(request),
        }
        calls.append(call)
    return calls


def matched_headers(headers: dict[str, str] | None, replied: dict[str, str]) -> Json:
    found: dict[str, Json] = {}
    if headers is None:
        return found
    wanted = {name.lower() for name in replied}
    for name, value in headers.items():
        if name.lower() in wanted:
            found[name.lower()] = value
    return found


def diagnostic_of(message: str) -> Json:
    if "provider not found" in message:
        return "provider"
    if "data field is required" in message:
        return "data"
    return None


# LiteLLM leaves the handler's params bare, so this typed view is the entry to create_variation
def variation_call(
    handler: VariationHandler, probe: Probe, supplied: openai.OpenAI | None
) -> ImageResponse:
    return handler.image_variations(
        ImageResponse(created=NOW),
        text_of(probe.api_key),
        text_of(probe.api_base),
        probe.model,
        file_object(probe),
        timeout_of(probe),
        "openai",
        logging_object(),
        dict(probe.optional_params),
        {},
        None,  # print_verbose stays at its default
        None,  # logger_fn stays at its default
        supplied,
        probe.organization,
        None,  # headers stay at their default
    )


def request_value(probe: Probe) -> Json:
    model = text_of(probe.model)
    config = variation_config(model)
    names: list[Json] = list(config.get_supported_openai_params(model))
    answer = empty_answer(names)
    upstream = Upstream(probe)
    transport = httpx.MockTransport(upstream.handle)
    supplied = supplied_client(probe, transport)
    previous_session = litellm.client_session
    if supplied is None:
        litellm.client_session = httpx.Client(transport=transport)
    # the handler reads its module's globals on every call; mypy forbids assigning an imported class by name
    if probe.missing_config:
        setattr(  # ast-grep-ignore: no-reflection
            variation_handler, CONFIG_MANAGER, AbsentProviderConfigManager
        )
    error: Json = None
    result: Json = None
    try:
        response = variation_call(OpenAIImageVariationsHandler(), probe, supplied)
    except PROVIDER_ERROR as failure:
        record = ErrorRecord.model_validate(failure, from_attributes=True)
        failure_view: dict[str, Json] = {
            "mapped": True,
            "status": record.status_code,
            "headers": matched_headers(record.headers, probe.reply.headers),
            "diagnostic": diagnostic_of(record.message),
        }
        error = failure_view
    else:
        result = DOCUMENT.validate_json(response.model_dump_json())
    finally:
        setattr(  # ast-grep-ignore: no-reflection
            variation_handler, CONFIG_MANAGER, ProviderConfigManager
        )
        litellm.client_session = previous_session
    answer["supplied"] = probe.client is not None
    answer["kind"] = "sync"
    answer["calls"] = calls_view(upstream.seen)
    answer["result"] = result
    answer["error"] = error
    return answer
