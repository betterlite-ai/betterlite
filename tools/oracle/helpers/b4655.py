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
PRISMA_NAME: Final = "litellm.proxy.proxy_server.prisma"
LOGGING_NAME: Final = "litellm.proxy.proxy_server.logging"
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
    service_logging_obj: ServiceLogging = ServiceLogging(f"{LOGGING_NAME}.service")


class PrismaClient(ModuleType):
    resolves: bool = True

    async def get_data(self, **kwargs: JsonValue) -> UserAPIKeyAuth:
        del kwargs
        if not self.resolves:
            raise LookupError("the session key does not resolve")
        return UserAPIKeyAuth()


class ProxyServer(ModuleType):
    master_key: str = ""
    prisma_client: ModuleType | None = None
    proxy_logging_obj: ModuleType | None = None
    user_api_key_cache: UserApiKeyCache | None = None


def install_proxy(master_key: str, *, resolves: bool) -> ModuleType | None:
    prisma = PrismaClient(PRISMA_NAME)
    prisma.resolves = resolves
    module = ProxyServer(PROXY_MODULE)
    module.master_key = master_key
    module.prisma_client = prisma
    module.proxy_logging_obj = ProxyLogging(LOGGING_NAME)
    module.user_api_key_cache = UserApiKeyCache()
    saved = sys.modules.get(PROXY_MODULE)
    sys.modules[PROXY_MODULE] = module
    return saved


def restore_proxy(saved: ModuleType | None) -> None:
    if saved is None:
        del sys.modules[PROXY_MODULE]
        return
    sys.modules[PROXY_MODULE] = saved


def run(input_value: Input) -> JsonValue:
    saved = install_proxy(input_value["master_key"], resolves=input_value["key_resolves"])
    try:
        headers: dict[str, str] = {}
        cookie_header = input_value["cookie_header"]
        if cookie_header is not None:
            headers["cookie"] = cookie_header
        client = TestClient(APP, follow_redirects=False)
        form: dict[str, str] = {
            "redirect_uri": CALLBACK,
            "code_challenge": "challenge",
            "api_key": "sk-oracle",
        }
        response = client.post(
            "/v1/mcp/oauth/authorize",
            data=form,
            headers=headers,
        )
        admitted = response.status_code == ADMITTED_STATUS
    finally:
        restore_proxy(saved)
    report: dict[str, JsonValue] = {"admitted": admitted}
    return JSON.validate_python(report)
