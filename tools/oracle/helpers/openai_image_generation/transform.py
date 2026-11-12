from collections.abc import Sequence
from typing import Final, Protocol, cast

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.openai.image_generation import get_openai_image_generation_config
from litellm.types.utils import ImageResponse
from pydantic import JsonValue as Json

from .probe import DOCUMENT, NOW, Probe
from .projection import Recorder, last

DALL_E_2_PARAMS: Final = ["n", "response_format", "quality", "size", "user"]
DALL_E_3_PARAMS: Final = ["n", "response_format", "quality", "size", "user", "style"]
GPT_IMAGE_PARAMS: Final = [
    "background",
    "moderation",
    "n",
    "output_compression",
    "output_format",
    "quality",
    "size",
    "user",
]


def family_name(supported: Sequence[str]) -> str:
    names = list(supported)
    if names == DALL_E_2_PARAMS:
        return "dall_e_2"
    if names == DALL_E_3_PARAMS:
        return "dall_e_3"
    if names == GPT_IMAGE_PARAMS:
        return "gpt_image"
    raise TypeError("unexpected image parameter list")


class Transform(Protocol):
    def __call__(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/image_generation/dall_e_2_transformation.py#L47
        self,
        *,
        model: str,
        raw_response: httpx.Response,
        model_response: ImageResponse,
        logging_obj: Logging,
        request_data: dict[str, Json],
        optional_params: dict[str, Json],
        litellm_params: dict[str, Json],
        encoding: None = None,
        api_key: str | None = None,
        json_mode: bool | None = None,
    ) -> ImageResponse: ...


def transform_value(probe: Probe) -> Json:
    config = get_openai_image_generation_config(probe.model)
    supported = config.get_supported_openai_params(model=probe.model)
    optional_params: dict[str, Json] = dict(probe.optional_params)
    request_data: dict[str, Json] = {
        "model": probe.model,
        "prompt": probe.prompt,
        **optional_params,
    }
    litellm_params: dict[str, Json] = {}
    recorder = Recorder()
    transform = cast(Transform, config.transform_image_generation_response)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/image_generation/dall_e_2_transformation.py#L47
    response = transform(
        model=probe.model,
        raw_response=httpx.Response(200, content=probe.reply.body.encode()),
        model_response=ImageResponse(created=NOW),
        logging_obj=cast(Logging, recorder),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/image_generation/dall_e_2_transformation.py#L53
        request_data=request_data,
        optional_params=optional_params,
        litellm_params=litellm_params,
        encoding=None,
    )
    dumped: dict[str, Json] = response.model_dump()
    return {
        "family": family_name(supported),
        "supported": cast(Json, supported),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L1178
        "transform": DOCUMENT.validate_python(dumped),
        "post_call": last(recorder.post_calls),
    }


def mapped_value(probe: Probe) -> Json:
    config = get_openai_image_generation_config(probe.model)
    supported = config.get_supported_openai_params(model=probe.model)
    optional_params: dict[str, Json] = dict(probe.existing_params)
    requested: dict[str, Json] = dict(probe.non_default_params)
    error: Json = None
    try:
        mapped = DOCUMENT.validate_python(
            config.map_openai_params(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/image_generation/dall_e_2_transformation.py#L26
                non_default_params=requested,
                optional_params=optional_params,
                model=probe.model,
                drop_params=probe.drop_params,
            )
        )
    except ValueError:
        mapped = DOCUMENT.validate_python(optional_params)
        error = {"kind": "unsupported_parameter"}
    return {
        "family": family_name(supported),
        "supported": cast(Json, supported),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L1178
        "mapped": mapped,
        "error": error,
    }
