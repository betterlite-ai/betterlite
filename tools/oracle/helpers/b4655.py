import sys
from types import ModuleType
from typing import Final, TypedDict

from fastapi import FastAPI
from litellm.proxy._experimental.mcp_server import byok_oauth_endpoints
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from pydantic import JsonValue, TypeAdapter
from starlette.testclient import TestClient

APP: Final = FastAPI()
APP.include_router(byok_oauth_endpoints.router)
CALLBACK: Final = "http://127.0.0.1:1456/callback"
PROXY_MODULE: Final = "litellm.proxy.proxy_server"
ADMITTED_STATUS: Final = 302
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class Input(TypedDict):
    cookie_header: str | None
    master_key: str
    key_resolves: bool
    now: int


class ServiceLogging(ModuleType):
    async def async_service_success_hook(self, **kwargs: JsonValue) -> None:
        del kwargs


class ProxyLogging(ModuleType):
    service_logging_obj: ServiceLogging

    def __init__(self) -> None:
        super().__init__("litellm.proxy.proxy_server.logging")
        self.service_logging_obj = ServiceLogging("litellm.proxy.proxy_server.logging.service")


class PrismaClient(ModuleType):
    resolves: bool

    def __init__(self, *, resolves: bool) -> None:
        super().__init__("litellm.proxy.proxy_server.prisma")
        self.resolves = resolves

    async def get_data(self, **kwargs: JsonValue) -> UserAPIKeyAuth:
        del kwargs
        if not self.resolves:
            raise LookupError("the session key does not resolve")
        return UserAPIKeyAuth()


class ProxyServer(ModuleType):
    master_key: str
    prisma_client: PrismaClient
    proxy_logging_obj: ProxyLogging
    user_api_key_cache: UserApiKeyCache

    def __init__(self, master_key: str, *, resolves: bool) -> None:
        super().__init__(PROXY_MODULE)
        self.master_key = master_key
        self.prisma_client = PrismaClient(resolves=resolves)
        self.proxy_logging_obj = ProxyLogging()
        self.user_api_key_cache = UserApiKeyCache()


def run(input_value: Input) -> JsonValue:
    saved = sys.modules.get(PROXY_MODULE)
    sys.modules[PROXY_MODULE] = ProxyServer(
        input_value["master_key"], resolves=input_value["key_resolves"]
    )
    try:
        headers: dict[str, str] = {}
        cookie_header = input_value["cookie_header"]
        if cookie_header is not None:
            headers["cookie"] = cookie_header
        client = TestClient(APP, follow_redirects=False)
        response = client.post(
            "/v1/mcp/oauth/authorize",
            data={"redirect_uri": CALLBACK, "code_challenge": "challenge", "api_key": "sk-oracle"},
            headers=headers,
        )
        admitted = response.status_code == ADMITTED_STATUS
    finally:
        if saved is None:
            sys.modules.pop(PROXY_MODULE, None)
        else:
            sys.modules[PROXY_MODULE] = saved
    report: dict[str, JsonValue] = {"admitted": admitted}
    return JSON.validate_python(report)
