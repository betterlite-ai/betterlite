import json
import os
from collections.abc import Callable
from contextlib import ExitStack
from functools import partial
from typing import TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.integrations.gcs_bucket.gcs_bucket_base import GCSBucketBase
from litellm.secret_managers import google_secret_manager as module

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
VARIABLES = (
    "GOOGLE_SECRET_MANAGER_PROJECT_ID",
    "GOOGLE_SECRET_MANAGER_REFRESH_INTERVAL",
    "GOOGLE_SECRET_MANAGER_ALWAYS_READ_SECRET_MANAGER",
)
HEADERS = {"Authorization": "Bearer oracle-token", "Content-Type": "application/json"}


class _Response:
    def __init__(self, status: int, body: str) -> None:
        self.status_code = status
        self.text = body

    def json(self) -> Json:
        return cast(Json, json.loads(self.text))


class _Client:
    def __init__(self, responses: list[dict[str, Json]]) -> None:
        self.responses = list(responses)
        self.requests: list[Json] = []

    def get(self, url: str, headers: dict[str, str]) -> _Response:
        self.requests.append({"url": url, "headers": cast(Json, dict(headers))})
        answer = self.responses.pop(0)
        return _Response(cast(int, answer["status"]), cast(str, answer["body"]))


class _Native:
    def __init__(self, value: str | None) -> None:
        self.value = value

    def sync_read_secret(self, secret_name: str) -> str | None:
        del secret_name
        return self.value


def _native_resolver(value: str | None) -> Callable[..., _Native]:
    def resolve(*args: object) -> _Native:
        del args
        return _Native(value)

    return resolve


def _no_init(self: object) -> None:
    del self


def _headers(manager: object) -> dict[str, str]:
    del manager
    return dict(HEADERS)


def run(input_value: dict[str, Json]) -> Json:
    environment = cast(dict[str, str], input_value.get("environment", {}))
    client = _Client(cast(list[dict[str, Json]], input_value.get("responses", [])))
    clock = [0]
    scrubbed = {name: value for name, value in os.environ.items() if name not in VARIABLES}
    native = cast(dict[str, str | None] | None, input_value.get("native"))
    with ExitStack() as patches:
        patches.enter_context(patch.dict(os.environ, {**scrubbed, **environment}, clear=True))
        patches.enter_context(patch("litellm.proxy.proxy_server.premium_user", new=True))
        patches.enter_context(patch.object(litellm, "secret_manager_client", None))
        patches.enter_context(patch.object(litellm, "_key_management_system", None))
        patches.enter_context(patch.object(GCSBucketBase, "__init__", _no_init))
        patches.enter_context(
            patch.object(GCSBucketBase, "sync_construct_request_headers", _headers),
        )
        patches.enter_context(patch.object(module, "_get_httpx_client", lambda: client))
        patches.enter_context(
            patch.object(module, "InMemoryCache", partial(InMemoryCache, clock=lambda: clock[0])),
        )
        if native is not None:
            patches.enter_context(
                patch.object(
                    module,
                    "resolve_native_provider_reader",
                    _native_resolver(native["value"]),
                ),
            )
        try:
            manager = module.GoogleSecretManager(
                refresh_interval=cast(
                    int,
                    input_value.get("refresh_interval", module.SECRET_MANAGER_REFRESH_INTERVAL),
                ),
                always_read_secret_manager=cast(bool, input_value.get("always_read", False)),
            )
        except ValueError:
            return {"initialized": False}
        steps: list[Json] = []
        for step in cast(list[dict[str, Json]], input_value.get("steps", [])):
            if "advance" in step:
                clock[0] += cast(int, step["advance"])
                steps.append({"advanced": step["advance"]})
                continue
            try:
                value = manager.get_secret_from_google_secret_manager(cast(str, step["read"]))
            except (ValueError, AttributeError):
                steps.append({"error": True})
            else:
                steps.append({"value": value})
        return {
            "initialized": True,
            "refresh_interval_seconds": manager.cache.default_ttl,
            "always_read": manager.always_read_secret_manager,
            "steps": steps,
            "requests": client.requests,
        }
