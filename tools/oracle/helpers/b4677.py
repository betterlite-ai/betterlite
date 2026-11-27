import sys
import time
from collections.abc import Mapping
from types import ModuleType
from typing import Final, Literal, Protocol, TypedDict, cast

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
PRISMA_NAME: Final = "litellm.proxy.proxy_server.prisma"
ACCESS_ITEM_OPEN: Final = 'class="access-item"><span class="check">&#10003;</span>'
DURATION_OPEN: Final = "selDur('"
OK_STATUS: Final = 200
REDIRECT_STATUS: Final = 302
UUID_LENGTH: Final = 36
UUID_DASH_POSITIONS: Final = (8, 13, 18, 23)
UUID_VERSION_POSITION: Final = 14
HIDDEN_FIELDS: Final = (
    "client_id",
    "redirect_uri",
    "code_challenge",
    "code_challenge_method",
    "state",
    "server_id",
    "duration",
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
    transport: str
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
    now: float
    prefill_live: int
    prefill_expired: int


class Response(Protocol):
    @property
    def status_code(self) -> int: ...

    @property
    def headers(self) -> Mapping[str, str]: ...

    @property
    def text(self) -> str: ...


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


def install(name: str, module: ModuleType) -> ModuleType | None:
    saved = sys.modules.get(name)
    sys.modules[name] = module
    return saved


def restore(name: str, saved: ModuleType | None) -> None:
    if saved is None:
        sys.modules.pop(name, None)
        return
    sys.modules[name] = saved


def proxy_module(master_key: str) -> ModuleType:
    proxy = Proxy(PROXY_MODULE)
    proxy.master_key = master_key
    proxy.prisma_client = Prisma(PRISMA_NAME)
    proxy.user_api_key_cache = UserApiKeyCache()
    return proxy


def registry_module(servers: list[ServerSpec]) -> ModuleType:
    registry: dict[str, RegistryEntry] = {}
    for spec in servers:
        registry[spec["server_id"]] = RegistryEntry(spec)
    manager = Manager(MANAGER_MODULE, registry)
    return ManagerModule(MANAGER_MODULE, manager)


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


def help_href(html: str) -> str | None:
    begin = html.find('class="help-link"')
    if begin < 0:
        return None
    return between(html[begin:], 'href="', '"')


def access_texts(html: str) -> list[JsonValue]:
    found: list[JsonValue] = []
    rest = html
    while True:
        _, separator, tail = rest.partition(ACCESS_ITEM_OPEN)
        if not separator:
            return found
        text, separator, rest = tail.partition("</div>")
        if not separator:
            return found
        found.append(text)


def durations(html: str) -> list[JsonValue]:
    found: list[JsonValue] = []
    rest = html
    while True:
        _, separator, tail = rest.partition(DURATION_OPEN)
        if not separator:
            return found
        value, separator, rest = tail.partition("'")
        if not separator:
            return found
        found.append(value)


def page_facts(html: str) -> dict[str, JsonValue]:
    hidden: dict[str, JsonValue] = {}
    for field in HIDDEN_FIELDS:
        hidden[field] = hidden_value(html, field)
    steps: dict[str, JsonValue] = {
        "s1": 'id="s1"' in html,
        "s2": 'id="s2"' in html,
    }
    return {
        "title": between(html, "<title>", "</title>"),
        "heading": between(html, '<h2 class="step-title">', "<"),
        "initial": between(html, 'class="logo logo-s">', "<"),
        "hidden": hidden,
        "access_items": access_texts(html),
        "help_href": help_href(html),
        "durations": durations(html),
        "steps": steps,
        "api_key_required": 'name="api_key"' in html and "required" in html,
        "form_post": 'method="POST"' in html,
    }


def response_report(response: Response) -> dict[str, JsonValue]:
    return {
        "status": response.status_code,
        "content_type": response.headers.get("content-type"),
        "body": None,
        "page": None,
        "location": None,
        "code_is_uuid": None,
        "stored": None,
        "store_size": None,
    }


def get_report(input_value: Input) -> dict[str, JsonValue]:
    client = TestClient(APP, follow_redirects=False)
    response: Response = client.get(PATH, params=input_value["query"])
    report = response_report(response)
    if response.status_code == OK_STATUS:
        report["page"] = page_facts(response.text)
    else:
        report["body"] = JSON.validate_json(response.text)
    return report


def looks_like_uuid4(code: str) -> bool:
    if len(code) != UUID_LENGTH:
        return False
    for index, character in enumerate(code):
        if index in UUID_DASH_POSITIONS:
            if character != "-":
                return False
        elif index == UUID_VERSION_POSITION:
            if character != "4":
                return False
        elif character not in "0123456789abcdef":
            return False
    return True


def redirect_prefix(redirect_uri: str) -> str:
    separator = "&" if "?" in redirect_uri else "?"
    return f"{redirect_uri}{separator}code="


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
    report: dict[str, JsonValue], response: Response, form: FormInput, started: float
) -> None:
    location = response.headers.get("location")
    if location is None:
        raise ValueError("the accepted flow carried no location")
    prefix = redirect_prefix(form["redirect_uri"])
    if not location.startswith(prefix):
        raise ValueError("the location did not continue the redirect_uri")
    code = location[len(prefix):].partition("&state=")[0]
    report["location"] = location.replace(code, "<code>")
    report["code_is_uuid"] = looks_like_uuid4(code)
    report["stored"] = stored_facts(STORE[code], started)


def prefill_record(expires_at: float) -> StoredRecord:
    return {
        "api_key": "sk-prefill",
        "server_id": "",
        "code_challenge": "",
        "redirect_uri": "",
        "client_id": "",
        "user_id": "",
        "expires_at": expires_at,
    }


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
    response: Response = client.post(PATH, data=form, headers=headers)
    report = response_report(response)
    report["store_size"] = len(STORE)
    if response.status_code == REDIRECT_STATUS:
        fill_redirect(report, response, form, started)
    else:
        report["body"] = JSON.validate_json(response.text)
    return report


def post_run(input_value: Input) -> JsonValue:
    master_key = input_value["master_key"]
    if master_key is None:
        master_key = ""
    saved_proxy = install(PROXY_MODULE, proxy_module(master_key))
    saved_manager = install(MANAGER_MODULE, registry_module([]))
    try:
        report = post_report(input_value)
    finally:
        restore(MANAGER_MODULE, saved_manager)
        restore(PROXY_MODULE, saved_proxy)
    return JSON.validate_python(report)


def get_run(input_value: Input) -> JsonValue:
    servers = input_value["servers"]
    if servers is None:
        servers = []
    saved_manager = install(MANAGER_MODULE, registry_module(servers))
    try:
        report = get_report(input_value)
    finally:
        restore(MANAGER_MODULE, saved_manager)
    return JSON.validate_python(report)


def run(input_value: Input) -> JsonValue:
    if input_value["route"] == "get":
        return get_run(input_value)
    return post_run(input_value)
