import os
from typing import TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.llms.xai.common_utils import XAIModelInfo

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def resolve(value: dict[str, Json]) -> Json:
    api_key = cast(str | None, value.get("api_key"))
    model = cast(str, value["model"])
    headers = cast(dict[str, str], value.get("headers") or {})
    with (
        patch.object(litellm, "xai_key", value.get("xai_key")),
        patch.object(litellm, "api_key", value.get("generic_key")),
        patch.dict(os.environ, cast(dict[str, str], value.get("env") or {}), clear=True),
    ):
        info = XAIModelInfo()
        return {
            "chat_key": info.get_api_key(api_key),
            "legacy_key": info.get_api_key(api_key, legacy_generic_before_env=True),
            "api_base": info.get_api_base(cast(str | None, value.get("api_base"))),
            "headers": info.validate_environment(  # pyright: ignore[reportUnknownMemberType]
                dict(headers), model, [], {}, {}, api_key=api_key
            ),
            "base_model": info.get_base_model(model),
        }


def run(value: dict[str, Json]) -> Json:
    return {"calls": [resolve(call) for call in cast(list[dict[str, Json]], value["calls"])]}
