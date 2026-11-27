import asyncio
from collections.abc import Callable, Coroutine
from typing import Final, Protocol, cast

import httpx
from litellm.exceptions import Timeout
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.llms.bedrock.image_edit.handler import BedrockImageEdit
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.types.utils import ImageResponse
from pydantic import JsonValue

from .contracts import Fixture, Probe
from .recording import JSON, Recorder, cost_of, image_entries, shown_headers

HANDLER: Final[type[BedrockImageEdit]] = BedrockImageEdit
Client = HTTPHandler | AsyncHTTPHandler | None


class EditHandler(Protocol):
    def __call__(
        self,
        model: str,
        image: list[str],
        prompt: str | None,
        model_response: ImageResponse,
        optional_params: dict[str, JsonValue],
        logging_obj: Logging,
        timeout: float | None,
        *,
        aimage_edit: bool,
        api_base: str | None,
        extra_headers: dict[str, str] | None,
        client: Client,
        api_key: str | None,
    ) -> ImageResponse | Coroutine[None, None, ImageResponse]: ...


def client_for(probe: Probe, fixture: Fixture) -> HTTPHandler | AsyncHTTPHandler:
    transport = httpx.MockTransport(fixture.respond)
    match probe.supplied:
        case "sync":
            return HTTPHandler(client=httpx.Client(transport=transport))
        case "async":
            return AsyncHTTPHandler(transport=transport)
        case "none":
            raise ValueError("the oracle case supplies a client")


def perform(probe: Probe, logging: Logging) -> tuple[Fixture, ImageResponse]:
    fixture = Fixture(probe)
    edit = cast(EditHandler, HANDLER().image_edit)  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/image_edit/handler.py#L69-L135
    timeout = probe.timeout_ms
    pending = edit(
        probe.model,
        probe.images,
        probe.prompt,
        ImageResponse(),
        probe.optional_params,
        logging,
        None if timeout is None else timeout / 1000,
        aimage_edit=probe.aimage_edit,
        api_base=probe.api_base,
        extra_headers=probe.extra_headers,
        client=client_for(probe, fixture),
        api_key=probe.api_key,
    )
    match pending:
        case Coroutine():
            return fixture, asyncio.run(pending)
        case object():
            return fixture, pending


def prepare_answer(probe: Probe) -> JsonValue:
    logging = Recorder()
    fixture, _ = perform(probe, logging)
    request = fixture.requests[0]
    return {
        "endpoint_url": str(request.url),
        "headers": JSON.validate_python(shown_headers(request)),
        "body": request.content.decode("utf-8"),
        "data": JSON.validate_python(JSON.validate_json(request.content)),
        "remaining_params": JSON.validate_python(sorted(probe.optional_params)),
        "pre_call": logging.pre,
    }


def result_answer(probe: Probe) -> JsonValue:
    logging = Recorder()
    try:
        _, response = perform(probe, logging)
    except (BedrockError, Timeout) as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L356
        return {
            "ok": False,
            "status": error.status_code,
            "post_call": logging.post is not None,
        }
    except (httpx.HTTPError, TypeError, ValueError):
        return {"ok": False, "status": None, "post_call": logging.post is not None}
    return {
        "ok": True,
        "images": image_entries(response),
        "cost": cost_of(response),
        "post_call": logging.post,
    }


def error_answer(probe: Probe) -> JsonValue:
    logging = Recorder()
    try:
        perform(probe, logging)
    except (BedrockError, Timeout) as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L356
        status: int | None = error.status_code
    except (httpx.HTTPError, TypeError, ValueError):
        status = None
    else:
        return {"ok": True, "status": None, "post_call": True}
    return {"ok": False, "status": status, "post_call": logging.post is not None}


def model_error_answer(probe: Probe) -> JsonValue:
    try:
        perform(probe, Recorder())
    except ValueError as error:
        return {"ok": False, "status": None, "message": str(error)}
    else:
        return {"ok": True, "status": None, "message": None}


def selection_answer(probe: Probe) -> JsonValue:
    fixture, _ = perform(probe, Recorder())
    posted = str(fixture.requests[0].url)
    return {
        "factory": "none",
        "timeout_ms": None,
        "supplied_requests": len(fixture.requests),
        "factory_requests": 0,
        "posted_url": posted,
    }


ANSWERS: dict[str, Callable[[Probe], JsonValue]] = {
    "prepare": prepare_answer,
    "result": result_answer,
    "error": error_answer,
    "model_error": model_error_answer,
    "selection": selection_answer,
}
