from typing import Final

from litellm.llms.bedrock.common_utils import BedrockError
from litellm.types.utils import ImageResponse
from pydantic import JsonValue

from .fixtures import Fixture, edit_cost, images_of, request_view
from .models import Probe, Recorder, edit_handler, generation_handler

NOW: Final = 1_700_000_000


def first_header_name(headers: dict[str, str] | None) -> str | None:
    if headers is None:
        return None
    return next(iter(sorted(headers)), None)


def success_row(
    fixture: Fixture, logger: Recorder, images: list[JsonValue], cost: JsonValue
) -> JsonValue:
    return {
        "request": request_view(fixture),
        "ok": True,
        "status": None,
        "images": images,
        "cost": cost,
        "post_call": logger.post is not None,
    }


def failure_row(fixture: Fixture, logger: Recorder, status: int | None) -> JsonValue:
    empty: list[JsonValue] = []
    return {
        "request": request_view(fixture),
        "ok": False,
        "status": status,
        "images": empty,
        "cost": None,
        "post_call": logger.post is not None,
    }


def generation_prompt(probe: Probe) -> str:
    prompt = probe.prompt
    if prompt is None:
        raise ValueError("a generation probe carries a prompt")
    return prompt


def case_row(probe: Probe, *, edit: bool) -> JsonValue:
    fixture = Fixture(probe.reply, first_header_name(probe.extra_headers))
    logger = Recorder()
    if edit:
        try:
            response = edit_handler().image_edit(
                model=probe.model,
                image=probe.images,
                prompt=probe.prompt,
                model_response=ImageResponse(),
                optional_params=dict(probe.optional_params),
                logging_obj=logger,
                timeout=None,
                aimage_edit=False,
                api_base=None,
                extra_headers=probe.extra_headers,
                client=fixture.client(),
                api_key=None,
            )
        except BedrockError as error:
            return failure_row(fixture, logger, error.status_code)
        except ValueError:
            return failure_row(fixture, logger, None)
        return success_row(fixture, logger, images_of(response), edit_cost(response))
    prompt = generation_prompt(probe)
    try:
        response = generation_handler().image_generation(
            model=probe.model,
            prompt=prompt,
            model_response=ImageResponse(created=NOW),
            optional_params=dict(probe.optional_params),
            logging_obj=logger,
            timeout=None,
            aimg_generation=False,
            api_base=None,
            extra_headers=None,
            client=fixture.client(),
            api_key=None,
        )
    except BedrockError as error:
        return failure_row(fixture, logger, error.status_code)
    return success_row(fixture, logger, images_of(response), None)
