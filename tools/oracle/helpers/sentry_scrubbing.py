import json
import sys
from collections.abc import Callable, Mapping
from importlib import import_module
from types import ModuleType
from typing import TypeAlias, cast

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

# Pinned to sentry-sdk 2.21.0, the version litellm's own uv.lock locks this behavior
# against. The oracle's litellm checkout has no sentry-sdk installed, so the two
# constants and the EventScrubber shape sentry_scrubbing.py imports are stubbed below.
DEFAULT_DENYLIST = [
    "password",
    "passwd",
    "secret",
    "api_key",
    "apikey",
    "auth",
    "credentials",
    "mysql_pwd",
    "privatekey",
    "private_key",
    "token",
    "session",
    "csrftoken",
    "sessionid",
    "x_csrftoken",
    "x_forwarded_for",
    "set_cookie",
    "cookie",
    "authorization",
    "x_api_key",
    "aiohttp_session",
    "connect.sid",
    "csrf_token",
    "csrf",
    "_csrf",
    "_csrf_token",
    "PHPSESSID",
    "_session",
    "symfony",
    "user_session",
    "_xsrf",
    "XSRF-TOKEN",
]
DEFAULT_PII_DENYLIST = ["x_forwarded_for", "x_real_ip", "ip_address", "remote_addr"]


class _EventScrubber:
    def __init__(
        self,
        denylist: list[str] | None = None,
        recursive: bool = False,  # noqa: FBT001, FBT002
        send_default_pii: bool = False,  # noqa: FBT001, FBT002
        pii_denylist: list[str] | None = None,
    ) -> None:
        self.denylist = DEFAULT_DENYLIST.copy() if denylist is None else denylist
        if not send_default_pii:
            pii = DEFAULT_PII_DENYLIST.copy() if pii_denylist is None else pii_denylist
            self.denylist = [*self.denylist, *pii]
        self.denylist = [name.lower() for name in self.denylist]
        self.recursive = recursive


def _install_sentry_sdk_stub() -> None:
    if "sentry_sdk" in sys.modules:
        return
    scrubber_module = ModuleType("sentry_sdk.scrubber")
    scrubber_namespace = cast(dict[str, object], vars(scrubber_module))
    scrubber_namespace["DEFAULT_DENYLIST"] = DEFAULT_DENYLIST
    scrubber_namespace["DEFAULT_PII_DENYLIST"] = DEFAULT_PII_DENYLIST
    scrubber_namespace["EventScrubber"] = _EventScrubber
    types_module = ModuleType("sentry_sdk.types")
    sentry_sdk_module = ModuleType("sentry_sdk")
    sentry_sdk_namespace = cast(dict[str, object], vars(sentry_sdk_module))
    sentry_sdk_namespace["scrubber"] = scrubber_module
    sentry_sdk_namespace["types"] = types_module
    sys.modules["sentry_sdk"] = sentry_sdk_module
    sys.modules["sentry_sdk.scrubber"] = scrubber_module
    sys.modules["sentry_sdk.types"] = types_module


_install_sentry_sdk_stub()
_ss = import_module("litellm.litellm_core_utils.sentry_scrubbing")

build_string_scrubber = cast(Callable[[bool], Callable[[str], str]], _ss.build_string_scrubber)
scrub_json_strings = cast(Callable[[Json, Callable[[str], str]], Json], _ss.scrub_json_strings)
send_default_pii_from_env = cast(Callable[[Mapping[str, str]], bool], _ss.send_default_pii_from_env)
build_sentry_init_options = cast(
    Callable[[Mapping[str, str]], dict[str, Json]], _ss.build_sentry_init_options
)


def text(fields: dict[str, Json], name: str) -> str:
    value = fields.get(name)
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def event_value(fields: dict[str, Json]) -> Json:
    encoded = fields.get("event_json")
    event = fields.get("event")
    if encoded is not None:
        if event is not None or not isinstance(encoded, str):
            raise ValueError("select one event representation")
        try:
            return cast(Json, json.loads(encoded))
        except json.JSONDecodeError as error:
            raise ValueError("invalid JSON event") from error
    return event


def env(fields: dict[str, Json]) -> dict[str, str]:
    value = fields.get("env", {})
    if not isinstance(value, dict) or not all(isinstance(item, str) for item in value.values()):
        raise ValueError("env must be an object of strings")
    return cast(dict[str, str], value)


def scrubber_options(fields: dict[str, Json]) -> Json:
    options = build_sentry_init_options(env(fields))
    scrubber = cast("_EventScrubber", options["event_scrubber"])
    return {
        "dsn": options["dsn"],
        "traces_sample_rate": options["traces_sample_rate"],
        "sample_rate": options["sample_rate"],
        "send_default_pii": options["send_default_pii"],
        "environment": options["environment"],
        "event_scrubber": {
            "denylist": cast(Json, scrubber.denylist),
            "recursive": cast(Json, scrubber.recursive),
        },
    }


def call(fields: dict[str, Json]) -> Json:
    match fields.get("function"):
        case "scrub_text":
            scrub = build_string_scrubber(fields.get("send_default_pii") is True)
            return scrub(text(fields, "text"))
        case "scrub_event":
            scrub = build_string_scrubber(fields.get("send_default_pii") is True)
            return scrub_json_strings(event_value(fields), scrub)
        case "send_default_pii_from_env":
            return send_default_pii_from_env(env(fields))
        case "build_sentry_init_options":
            return scrubber_options(fields)
        case other:
            raise ValueError(f"unknown sentry_scrubbing function: {other!r}")


def run(input_value: dict[str, Json]) -> Json:
    return {"result": call(input_value)}
