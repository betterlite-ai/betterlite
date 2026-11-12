import asyncio
from collections.abc import Coroutine, Mapping
from typing import Protocol, cast

import httpx
import litellm
import openai
from litellm.litellm_core_utils.core_helpers import (
    get_provider_response_headers_from_hidden_params,
)
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.openai.common_utils import OpenAIError
from litellm.llms.openai.openai import OpenAIChatCompletion
from litellm.types.utils import ImageResponse
from pydantic import JsonValue as Json

from .fixture import Client, Fixture, timeout_value
from .probe import DOCUMENT, NOW, Probe
from .projection import Recorder, last

litellm.suppress_debug_info = True


class Generate(Protocol):
    def __call__(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/openai.py#L1456
        self,
        *,
        model: str,
        prompt: str,
        timeout: float,
        optional_params: dict[str, Json],
        logging_obj: Logging,
        api_key: str | None = None,
        api_base: str | None = None,
        model_response: ImageResponse | None = None,
        client: Client | None = None,
        aimg_generation: bool | None = None,
        organization: str | None = None,
        headers: dict[str, str] | None = None,
    ) -> ImageResponse: ...


def invoke(probe: Probe, client: Client, recorder: Recorder) -> ImageResponse:
    completion = OpenAIChatCompletion()
    generate = cast(Generate, completion.image_generation)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/openai.py#L1456
    optional_params: dict[str, Json] = dict(probe.optional_params)
    outcome = generate(
        model=probe.model,
        prompt=probe.prompt,
        timeout=timeout_value(probe),
        optional_params=optional_params,
        logging_obj=cast(Logging, recorder),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/openai.py#L1398
        api_key=probe.api_key,
        api_base=probe.api_base,
        model_response=ImageResponse(created=NOW),
        client=client,
        aimg_generation=probe.is_async,
        organization=probe.organization,
        headers=probe.headers,
    )
    if probe.is_async:
        return asyncio.run(
            cast(Coroutine[None, None, ImageResponse], outcome)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/openai.py#L1479
        )
    return outcome


def client_value(client: Client) -> Json:
    return {
        "api_key": client.api_key,
        "api_base": client.base_url,
        "organization": client.organization,
        "max_retries": client.max_retries,
    }


def request_value(probe: Probe) -> Json:
    fixture = Fixture(probe)
    recorder = Recorder()
    client = Client(fixture, probe)
    error: Json = None
    result: Json = None
    provider_headers: Json = None
    try:
        response = invoke(probe, client, recorder)
    except OpenAIError as failure:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/openai.py#L1381
        error = {"mapped": True, "status": failure.status_code}
    except openai.APIStatusError as failure:
        error = {"mapped": False, "status": failure.status_code}
    except (httpx.HTTPError, ValueError, TypeError):
        error = {"mapped": False, "status": None}
    else:
        dumped: dict[str, Json] = response.model_dump()
        result = DOCUMENT.validate_python(dumped)
        found: Mapping[str, str] | None = get_provider_response_headers_from_hidden_params(
            response
        )
        if found is not None:
            provider_headers = DOCUMENT.validate_python(found)
    calls: list[Json] = [
        {"body": call.body, "timeout": call.timeout} for call in fixture.calls
    ]
    return {
        "client": None if not calls else client_value(client),
        "calls": calls,
        "result": result,
        "provider_headers": provider_headers,
        "error": error,
        "pre_call": last(recorder.pre_calls),
        "post_call": last(recorder.post_calls),
    }
