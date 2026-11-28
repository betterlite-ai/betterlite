import re
import sys
import time
from collections.abc import Mapping
from types import ModuleType
from typing import Final, Literal, TypedDict, cast

from fastapi import FastAPI
from litellm.proxy._experimental.mcp_server import byok_oauth_endpoints
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from pydantic import JsonValue, TypeAdapter
from starlette.testclient import TestClient

APP: Final = FastAPI()
APP.include_router(byok_oauth_endpoints.router)
PATH: Final = "/v1/mcp/oauth/authorize"
PROXY_MODULE: Final = "litellm.proxy.proxy_server"
MANAGER_MODULE: Final = "litellm.proxy._experimental.mcp_server.mcp_server_manager"
ACCESS_ITEM_OPEN: Final = 'class="access-item"><span class="check">&#10003;</span>'
DURATION_OPEN: Final = "selDur('"
OK_STATUS: Final = 200
REDIRECT_STATUS: Final = 302
UUID4: Final = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[0-9a-f]{4}-[0-9a-f]{12}")
HIDDEN_FIELDS: Final = (
    "client_id", "redirect_uri", "code_challenge", "code_challenge_method", "state",
    "server_id", "duration",
)
REPORT_KEYS: Final = (
    "status", "content_type", "body", "page",
    "location", "code_is_uuid", "stored", "store_size",
)
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class StoredRecord(TypedDict):
    api_key: str
    server_id: str
    code_challenge: str
    redirect_uri: str
    client_id: str
    user_id: str
    expires_at: float


class ServerSpec(TypedDict):
    server_id: str
    name: str
    server_name: str | None
    byok_description: list[str] | None
    byok_api_key_help_url: str | None


class FormInput(TypedDict):
    client_id: str
    redirect_uri: str
    code_challenge: str
    code_challenge_method: str
    state: str
    server_id: str
    api_key: str


class Input(TypedDict):
    route: Literal["get", "post"]
    query: dict[str, str] | None
    servers: list[ServerSpec] | None
    form: FormInput | None
    cookie_header: str | None
    master_key: str | None
    prefill_live: int
    prefill_expired: int


class RegistryEntry:
    def __init__(self, spec: ServerSpec) -> None:
        self.server_name = spec["server_name"]
        self.name = spec["name"]
        self.byok_description = spec["byok_description"]
        self.byok_api_key_help_url = spec["byok_api_key_help_url"]


class Manager(ModuleType):
    def __init__(self, name: str, registry: Mapping[str, RegistryEntry]) -> None:
        super().__init__(name)
        self.registry = registry

    def get_registry(self) -> Mapping[str, RegistryEntry]:
        return self.registry


class ManagerModule(ModuleType):
    def __init__(self, name: str, manager: Manager) -> None:
        super().__init__(name)
        self.global_mcp_server_manager = manager


class Prisma(ModuleType):
    async def get_data(self, **_: JsonValue) -> None:
        return None


class Proxy(ModuleType):
    master_key: str = ""
    prisma_client: ModuleType | None = None
    proxy_logging_obj: ModuleType | None = None
    user_api_key_cache: UserApiKeyCache | None = None


STORE: Final = cast(dict[str, StoredRecord], byok_oauth_endpoints._byok_auth_codes)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/byok_oauth_endpoints.py#L43


def proxy_module(master_key: str) -> ModuleType:
    proxy = Proxy(PROXY_MODULE)
    proxy.master_key = master_key
    proxy.prisma_client = Prisma("litellm.proxy.proxy_server.prisma")
    proxy.user_api_key_cache = UserApiKeyCache()
    return proxy


def registry_module(servers: list[ServerSpec]) -> ModuleType:
    registry: dict[str, RegistryEntry] = {}
    for spec in servers:
        registry[spec["server_id"]] = RegistryEntry(spec)
    return ManagerModule(MANAGER_MODULE, Manager(MANAGER_MODULE, registry))


def between(text: str, start: str, end: str) -> str | None:
    begin = text.find(start)
    if begin < 0:
        return None
    stop = text.find(end, begin + len(start))
    if stop < 0:
        return None
    return text[begin + len(start):stop]


def hidden_value(html: str, field: str) -> str | None:
    marker = f'name="{field}"'
    begin = html.find(marker)
    if begin < 0:
        return None
    return between(html[begin + len(marker):], 'value="', '"')


def marked_texts(html: str, opening: str, closing: str) -> list[JsonValue]:
    found: list[JsonValue] = []
    rest = html
    while True:
        _, separator, tail = rest.partition(opening)
        if not separator:
            return found
        text, separator, rest = tail.partition(closing)
        if not separator:
            return found
        found.append(text)


def page_facts(html: str) -> dict[str, JsonValue]:
    hidden: dict[str, JsonValue] = {
        field: hidden_value(html, field) for field in HIDDEN_FIELDS
    }
    help_at = html.find('class="help-link"')
    help_href = None if help_at < 0 else between(html[help_at:], 'href="', '"')
    return {
        "title": between(html, "<title>", "</title>"),
        "heading": between(html, '<h2 class="step-title">', "<"),
        "initial": between(html, 'class="logo logo-s">', "<"),
        "hidden": hidden,
        "access_items": marked_texts(html, ACCESS_ITEM_OPEN, "</div>"),
        "help_href": help_href,
        "durations": marked_texts(html, DURATION_OPEN, "'"),
        "steps": {"s1": 'id="s1"' in html, "s2": 'id="s2"' in html},
        "api_key_required": 'name="api_key"' in html and "required" in html,
        "form_post": 'method="POST"' in html,
    }


def response_report(status: int, content_type: str | None) -> dict[str, JsonValue]:
    report: dict[str, JsonValue] = dict.fromkeys(REPORT_KEYS)
    report["status"] = status
    report["content_type"] = content_type
    return report


def get_report(input_value: Input) -> dict[str, JsonValue]:
    client = TestClient(APP, follow_redirects=False)
    response = client.request("GET", PATH, params=input_value["query"])
    report = response_report(response.status_code, response.headers.get("content-type"))
    if response.status_code == OK_STATUS:
        report["page"] = page_facts(response.text)
    else:
        report["body"] = JSON.validate_json(response.text)
    return report


def stored_facts(record: StoredRecord, started: float) -> dict[str, JsonValue]:
    return {
        "api_key": record["api_key"],
        "server_id": record["server_id"],
        "code_challenge": record["code_challenge"],
        "redirect_uri": record["redirect_uri"],
        "client_id": record["client_id"],
        "user_id": record["user_id"],
        "expires_in": float(round(record["expires_at"] - started)),
    }


def fill_redirect(
    report: dict[str, JsonValue], location: str | None, form: FormInput, started: float
) -> None:
    if location is None:
        raise ValueError("the accepted flow carried no location")
    redirect_uri = form["redirect_uri"]
    separator = "&" if "?" in redirect_uri else "?"
    prefix = f"{redirect_uri}{separator}code="
    if not location.startswith(prefix):
        raise ValueError("the location did not continue the redirect_uri")
    code = location[len(prefix):].partition("&state=")[0]
    report["location"] = location.replace(code, "<code>")
    report["code_is_uuid"] = UUID4.fullmatch(code) is not None
    report["stored"] = stored_facts(STORE[code], started)


def prefill_record(expires_at: float) -> StoredRecord:
    return StoredRecord(
        api_key="sk-prefill",
        server_id="",
        code_challenge="",
        redirect_uri="",
        client_id="",
        user_id="",
        expires_at=expires_at,
    )


def prefill(live: int, expired: int, now: float) -> None:
    for index in range(live):
        STORE[f"live-{index}"] = prefill_record(now + 300.0)
    for index in range(expired):
        STORE[f"expired-{index}"] = prefill_record(now - 1000.0)


def post_report(input_value: Input) -> dict[str, JsonValue]:
    form = input_value["form"]
    if form is None:
        raise ValueError("a post case needs a form")
    STORE.clear()
    started = time.time()
    prefill(input_value["prefill_live"], input_value["prefill_expired"], started)
    headers: dict[str, str] = {}
    cookie_header = input_value["cookie_header"]
    if cookie_header is not None:
        headers["cookie"] = cookie_header
    client = TestClient(APP, follow_redirects=False)
    response = client.post(PATH, data=form, headers=headers)
    report = response_report(response.status_code, response.headers.get("content-type"))
    report["store_size"] = len(STORE)
    if response.status_code == REDIRECT_STATUS:
        fill_redirect(report, response.headers.get("location"), form, started)
    else:
        report["body"] = JSON.validate_json(response.text)
    return report


def run(input_value: Input) -> JsonValue:
    saved = sys.modules.copy()
    try:
        if input_value["route"] == "get":
            servers = input_value["servers"]
            if servers is None:
                servers = []
            sys.modules[MANAGER_MODULE] = registry_module(servers)
            report = get_report(input_value)
        else:
            master_key = input_value["master_key"]
            if master_key is None:
                master_key = ""
            sys.modules[PROXY_MODULE] = proxy_module(master_key)
            sys.modules[MANAGER_MODULE] = registry_module([])
            report = post_report(input_value)
    finally:
        sys.modules.clear()
        sys.modules.update(saved)
    return JSON.validate_python(report)
