import os
from typing import TypeAlias, cast
from unittest.mock import patch

import httpx
import litellm
from litellm.llms.xai.common_utils import XAIModelInfo

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class RecordingClient:
    def __init__(self, status: int, body: str) -> None:
        self.status = status
        self.body = body
        self.urls: list[str] = []
        self.authorizations: list[str] = []

    def get(self, url: str, headers: dict[str, str]) -> httpx.Response:
        self.urls.append(url)
        self.authorizations.append(headers["Authorization"])
        return httpx.Response(self.status, content=self.body, request=httpx.Request("GET", url))


def discover(api_key: str | None, api_base: str | None, reply: dict[str, Json] | None) -> Json:
    if reply is None:
        return None
    client = RecordingClient(cast(int, reply["status"]), cast(str, reply["body"]))
    with patch.object(litellm, "module_level_client", client):
        try:
            ids = XAIModelInfo().get_models(api_key, api_base)
        except (KeyError, TypeError):
            return {"error": "malformed"}
        except ValueError as error:
            kind = "missing_credentials" if "is not set" in str(error) else "malformed"
            return {"error": kind}
        except Exception as error:  # get_models raises a bare Exception for HTTP errors
            if str(error).startswith("Failed to fetch models from XAI"):
                return {"error": "status"}
            raise
    return {
        "url": client.urls[0],
        "authorization": client.authorizations[0],
        "ids": cast(Json, ids),
    }


def resolve(value: dict[str, Json]) -> Json:
    api_key = cast(str | None, value.get("api_key"))
    api_base = cast(str | None, value.get("api_base"))
    model = cast(str, value["model"])
    env = cast(dict[str, str], value.get("env") or {})
    given = cast(dict[str, str], value.get("headers") or {})
    with (
        patch.object(litellm, "xai_key", value.get("xai_key")),
        patch.object(litellm, "api_key", value.get("generic_key")),
        patch.dict(os.environ, env, clear=True),
    ):
        info = XAIModelInfo()
        return {
            "chat_key": info.get_api_key(api_key),
            "legacy_key": info.get_api_key(api_key, legacy_generic_before_env=True),
            "api_base": info.get_api_base(api_base),
            "headers": info.validate_environment(  # pyright: ignore[reportUnknownMemberType]
                dict(given), model, [], {}, {}, api_key=api_key
            ),
            "base_model": info.get_base_model(model),
            "models": discover(api_key, api_base, cast(dict[str, Json] | None, value.get("reply"))),
        }


def run(value: dict[str, Json]) -> Json:
    calls = cast(list[dict[str, Json]], value["calls"])
    return {"calls": [resolve(call) for call in calls]}
