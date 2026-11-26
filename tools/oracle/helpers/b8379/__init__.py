import asyncio
from collections.abc import Mapping
from typing import Final, Literal, Protocol, TypedDict

from litellm.proxy import proxy_server
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.proxy_server import ProxyConfig
from litellm.proxy.utils import PrismaClient, evict_config_param
from pydantic import JsonValue, TypeAdapter

CONFIG: Final = TypeAdapter(dict[str, JsonValue])


class Input(TypedDict):
    section: Literal["general_settings", "litellm_settings"]
    row: JsonValue


class ConfigRow:
    def __init__(self, param_name: str, param_value: JsonValue) -> None:
        self.param_name = param_name
        self.param_value = param_value


class ConfigTable:
    def __init__(self, rows: Mapping[str, JsonValue]) -> None:
        self.rows = rows

    async def find_first(self, *, where: Mapping[str, str]) -> ConfigRow | None:
        name = where["param_name"]
        if name not in self.rows:
            return None
        return ConfigRow(name, self.rows[name])


class CaseDb(PrismaWrapper):
    def __init__(self, table: ConfigTable) -> None:
        self.litellm_config = table


class CaseClient(PrismaClient):
    def __init__(self, table: ConfigTable) -> None:
        self.db = CaseDb(table)


# get_config is annotated with a bare dict; this states the mapping the reload returns
class Reloader(Protocol):
    async def get_config(self, config_file_path: str | None = None) -> Mapping[str, JsonValue]: ...


async def reload(input_value: Input, reloader: Reloader) -> dict[str, JsonValue]:
    names = ("general_settings", "router_settings", "litellm_settings", "environment_variables")
    for name in names:
        await evict_config_param(name)
    proxy_server.prisma_client = CaseClient(
        ConfigTable({input_value["section"]: input_value["row"]})
    )
    proxy_server.store_model_in_db = True
    proxy_server.user_config_file_path = None
    return CONFIG.validate_python(await reloader.get_config())


def run(input_value: Input) -> JsonValue:
    config = asyncio.run(reload(input_value, ProxyConfig()))
    return config[input_value["section"]]
