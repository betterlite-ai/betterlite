from typing import Final, cast

import litellm
from pydantic import JsonValue

from azure_resource_urls import Input as ResourceUrlsInput
from azure_resource_urls import run as resource_urls_run

from .contracts import FIELDS, ContainerVideoVectorCase

litellm.suppress_debug_info = True

CONTAINER_KEY: Final[str] = "container_api_version_query"
VIDEO_KEY: Final[str] = "video_supported_fields"


def reports_of(steps: list[JsonValue], key: str) -> JsonValue:
    payload: dict[str, JsonValue] = {key: steps}
    leaf_input = cast(ResourceUrlsInput, payload)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/videos/main.py#L76
    answer = resource_urls_run(leaf_input)
    return FIELDS.validate_python(answer)["steps"]


def container_video_vector(case: ContainerVideoVectorCase) -> JsonValue:
    urls = reports_of(case["urls"], CONTAINER_KEY)
    video = reports_of(case["video"], VIDEO_KEY)
    return {"urls": urls, "video": video}
