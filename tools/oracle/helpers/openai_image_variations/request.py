import email.policy
from email.message import EmailMessage
from email.parser import BytesParser
from typing import Final

import httpx
import litellm
import openai
from litellm.llms.openai.common_utils import OpenAIError
from litellm.llms.openai.image_variations.handler import OpenAIImageVariationsHandler
from litellm.llms.openai.image_variations.transformation import OpenAIImageVariationConfig
from litellm.types.utils import ImageResponse, LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import JsonValue as Json

from .probe import DOCUMENT, NOW, Probe, file_object
from .transform import empty_answer, supported_value

litellm.suppress_debug_info = True
REAL_CONFIG: Final = ProviderConfigManager.get_provider_image_variation_config


def absent_config(model: str, provider: LlmProviders) -> OpenAIImageVariationConfig | None:
    return None


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
                return httpx.Response(
                    self.probe.reply.status,
                    headers=self.probe.reply.headers,
                    content=self.probe.reply.body.encode(),
                )


class Recorder:
    def pre_call(self, **kwargs: Json) -> None:
        pass

    def post_call(self, **kwargs: Json) -> None:
        pass


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


def part_view(part: EmailMessage) -> Json:
    match part.get_payload(decode=True):
        case bytes() as payload:
            data = payload.decode("utf-8")
        case None | str():
            raise TypeError("a multipart part carries bytes")
    return {
        "name": part.get_param("name", header="content-disposition"),
        "filename": part.get_filename(),
        "content_type": part.get("content-type"),
        "data": data,
    }


def parts_view(request: httpx.Request) -> list[Json]:
    content_type = request.headers.get("content-type")
    if content_type is None:
        raise ValueError("a multipart request carries a content type")
    raw = b"Content-Type: " + content_type.encode() + b"\r\nMIME-Version: 1.0\r\n\r\n" + request.read()
    message = BytesParser(policy=email.policy.default).parsebytes(raw)
    return [part_view(part) for part in message.iter_parts()]


def invoke(probe: Probe, client: openai.OpenAI | None, recorder: Recorder) -> ImageResponse:
    return OpenAIImageVariationsHandler().image_variations(
        model_response=ImageResponse(created=NOW),
        api_key=probe.api_key or "",
        api_base=probe.api_base or "",
        model=probe.model,
        image=file_object(probe),
        timeout=float(probe.timeout) if probe.timeout is not None else None,
        custom_llm_provider="openai",
        logging_obj=recorder,
        optional_params=dict(probe.optional_params),
        litellm_params={},
        client=client,
        organization=probe.organization,
        headers=None,
    )


def request_value(probe: Probe) -> Json:
    upstream = Upstream(probe)
    transport = httpx.MockTransport(upstream.handle)
    recorder = Recorder()
    client = supplied_client(probe, transport)
    previous = litellm.client_session
    if client is None:
        litellm.client_session = httpx.Client(transport=transport)
    if probe.missing_config:
        ProviderConfigManager.get_provider_image_variation_config = absent_config
    error: Json = None
    result: Json = None
    try:
        response = invoke(probe, client, recorder)
    except OpenAIError as failure:
        supplied = {name.lower() for name in probe.reply.headers}
        found: dict[str, Json] = {}
        if failure.headers is not None:
            for name, value in failure.headers.items():
                if name.lower() in supplied:
                    found[name.lower()] = value
        diagnostic: Json = None
        match failure.message:
            case message if "provider not found" in message:
                diagnostic = "provider"
            case message if "data field is required" in message:
                diagnostic = "data"
            case _:
                diagnostic = None
        error = {"mapped": True, "status": failure.status_code, "headers": found, "diagnostic": diagnostic}
    else:
        result = DOCUMENT.validate_python(response.model_dump())
    finally:
        litellm.client_session = previous
        ProviderConfigManager.get_provider_image_variation_config = REAL_CONFIG
    answer = empty_answer(supported_value(OpenAIImageVariationConfig(), probe.model or ""))
    answer["supplied"] = probe.client is not None
    answer["kind"] = "sync"
    answer["calls"] = [
        {
            "url": str(request.url),
            "authorization": request.headers.get("authorization"),
            "organization": request.headers.get("openai-organization"),
            "parts": parts_view(request),
        }
        for request in upstream.seen
    ]
    answer["result"] = result
    answer["error"] = error
    return answer
