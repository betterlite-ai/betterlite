import asyncio
import os
import sys
from collections.abc import Generator
from contextlib import contextmanager
from dataclasses import dataclass
from types import ModuleType
from typing import Final, TypedDict

from litellm.proxy.management_endpoints.cache_settings_endpoints import get_cache_settings
from litellm.proxy.proxy_server import ProxyConfig
from pydantic import JsonValue, TypeAdapter

PROXY_MODULE: Final = "litellm.proxy.proxy_server"
ROW_ID: Final = "cache_config"


class SettingsResponse(TypedDict):
    current_values: dict[str, JsonValue]


class Input(TypedDict):
    stored: JsonValue
    environment: dict[str, str]


RESPONSE: Final = TypeAdapter(SettingsResponse)


@dataclass
class CacheConfigRow:
    cache_settings: JsonValue


@dataclass
class FakeTable:
    settings: JsonValue

    async def find_unique(self, where: dict[str, str]) -> CacheConfigRow | None:
        await asyncio.sleep(0)
        if where["id"] != ROW_ID:
            raise ValueError(f"the cache settings row id is {ROW_ID}, not {where['id']}")
        return CacheConfigRow(cache_settings=self.settings)


@dataclass
class FakeDatabase:
    litellm_cacheconfig: FakeTable


@dataclass
class FakePrisma:
    db: FakeDatabase


class ProxyServer(ModuleType):
    prisma_client: FakePrisma | None = None
    proxy_config: ProxyConfig | None = None
    master_key: str | None = None


def check_environment(environment: dict[str, str]) -> None:
    for name, value in environment.items():
        if os.environ.get(name) != value:
            raise ValueError(f"the case env and the helper input disagree on {name}")
    declared = set(environment)
    undeclared = sorted(
        {name for name in os.environ if name.startswith("REDIS_")} - declared,
    )
    if undeclared:
        raise ValueError(f"the case env sets variables the helper input omits: {undeclared}")


@contextmanager
def fake_proxy(settings: JsonValue) -> Generator[None]:
    proxy_server = ProxyServer(PROXY_MODULE)
    proxy_server.prisma_client = FakePrisma(
        db=FakeDatabase(litellm_cacheconfig=FakeTable(settings)),
    )
    proxy_server.proxy_config = ProxyConfig()
    saved = sys.modules.get(PROXY_MODULE)
    sys.modules[PROXY_MODULE] = proxy_server
    try:
        yield
    finally:
        if saved is None:
            _ = sys.modules.pop(PROXY_MODULE, None)
        else:
            sys.modules[PROXY_MODULE] = saved


def settings_answer(input_value: Input) -> JsonValue:
    with fake_proxy(input_value["stored"]):
        response = RESPONSE.validate_json(asyncio.run(get_cache_settings()).model_dump_json())  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/cache_settings_endpoints.py#L448
        return response["current_values"]


def run(input_value: Input) -> JsonValue:
    check_environment(input_value["environment"])
    return settings_answer(input_value)
