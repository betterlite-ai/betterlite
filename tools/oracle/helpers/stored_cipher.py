import base64
import logging
import os
import sys
from collections.abc import Callable, Generator
from contextlib import contextmanager
from types import ModuleType
from typing import TypeAlias, cast, override

import nacl.secret
from cryptography.exceptions import InvalidTag
from litellm.litellm_core_utils.app_crypto import AppCrypto
from litellm.proxy.common_utils import callback_utils
from litellm.proxy.spend_tracking import spend_tracking_utils

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class DropAll(logging.Filter):
    @override
    def filter(self, record: logging.LogRecord) -> bool:
        return False


@contextmanager
def silenced_proxy_logger() -> Generator[None]:
    drop = DropAll()
    logger = logging.getLogger("LiteLLM Proxy")
    logger.filters.insert(0, drop)
    try:
        yield
    finally:
        logger.removeFilter(drop)


class FixedRandom:
    def __init__(self, nonce: bytes) -> None:
        self.nonce = nonce

    def __call__(self, size: int) -> bytes:
        return self.nonce[:size]


def text(step: dict[str, Json], name: str) -> str:
    value = step.get(name)
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def optional_text(step: dict[str, Json], name: str) -> str | None:
    value = step.get(name)
    if value is not None and not isinstance(value, str):
        raise ValueError(f"{name} must be a string or null")
    return value


@contextmanager
def restored_globals(
    salt_key: str | None,
    master_key: str | None,
    general_settings: Json,
    nonce_hex: str,
) -> Generator[None]:
    fixed = FixedRandom(bytes.fromhex(nonce_hex))
    # nacl.secret binds random at import, so the box only sees a replacement in its own namespace
    box_namespace = cast(dict[str, object], vars(nacl.secret))
    saved_box_random = box_namespace["random"]
    saved_urandom = os.urandom
    saved_salt = os.environ.pop("LITELLM_SALT_KEY", None)
    logger = logging.getLogger("LiteLLM Proxy")
    saved_level = logger.level
    if salt_key is not None:
        os.environ["LITELLM_SALT_KEY"] = salt_key
    # the codec reads only these two globals, and the real module adds seconds to every helper call
    proxy_server = ModuleType("litellm.proxy.proxy_server")
    proxy_namespace = cast(dict[str, Json], vars(proxy_server))
    proxy_namespace["master_key"] = master_key
    if isinstance(general_settings, dict):
        # a missing global makes LiteLLM's import fail, its "settings unavailable" path
        proxy_namespace["general_settings"] = general_settings
    saved_proxy_server = sys.modules.get(proxy_server.__name__)
    sys.modules[proxy_server.__name__] = proxy_server
    os.urandom = box_namespace["random"] = fixed
    logger.setLevel(logging.DEBUG)
    try:
        yield
    finally:
        logger.setLevel(saved_level)
        if saved_proxy_server is None:
            del sys.modules[proxy_server.__name__]
        else:
            sys.modules[proxy_server.__name__] = saved_proxy_server
        os.urandom, box_namespace["random"] = saved_urandom, saved_box_random
        os.environ.pop("LITELLM_SALT_KEY", None)
        if saved_salt is not None:
            os.environ["LITELLM_SALT_KEY"] = saved_salt


def app_aad(step: dict[str, Json], name: str) -> bytes | None:
    value = optional_text(step, name)
    return value.encode("utf-8") if value is not None else None


def app_encrypt(app: AppCrypto, data: dict[str, Json], aad: bytes | None) -> dict[str, Json]:
    encrypt = cast(Callable[[dict[str, Json], bytes | None], dict[str, Json]], app.encrypt_json)
    return encrypt(data, aad)


def call_app_crypto(
    decrypt: Callable[[dict[str, Json], bytes | None], Json],
    app: AppCrypto,
    step: dict[str, Json],
) -> Json:
    match text(step, "kind"):
        case "roundtrip":
            aad = app_aad(step, "aad")
            envelope = app_encrypt(app, cast(dict[str, Json], step.get("data")), aad)
            return decrypt(envelope, aad)
        case "aad_mismatch":
            data = cast(dict[str, Json], step.get("data"))
            envelope = app_encrypt(app, data, app_aad(step, "encrypt_aad"))
            return decrypt(envelope, app_aad(step, "decrypt_aad"))
        case "decrypt":
            envelope_value = step.get("envelope")
            if not isinstance(envelope_value, dict):
                raise ValueError("envelope must be an object")
            return decrypt(cast(dict[str, Json], envelope_value), app_aad(step, "aad"))
        case other:
            raise ValueError(f"unknown app_crypto step: {other}")


def run_app_crypto(input_value: dict[str, Json]) -> Json:
    app = AppCrypto(base64.b64decode(text(input_value, "key")))
    decrypt = cast(Callable[[dict[str, Json], bytes | None], Json], app.decrypt_json)
    steps = input_value["steps"]
    if not isinstance(steps, list):
        raise ValueError("steps must be a list")
    results: list[Json] = []
    for step in steps:
        if not isinstance(step, dict):
            raise ValueError("each step must be an object")
        try:
            results.append({"value": call_app_crypto(decrypt, app, step)})
        except (ValueError, KeyError, TypeError, InvalidTag) as error:
            results.append({"error": {"class": type(error).__name__}})
    return results


def call_callback(step: dict[str, Json], extra: set[str]) -> Json:
    metadata = step.get("metadata")
    match text(step, "kind"):
        case "encrypt":
            return cast(Json, callback_utils.encrypt_callback_vars(metadata))
        case "decrypt":
            return cast(Json, callback_utils.decrypt_callback_vars(metadata))
        case "sensitive":
            return callback_utils.is_sensitive_callback_key(text(step, "key"), extra)
        case other:
            raise ValueError(f"unknown callback_vars step: {other}")


def run_callback_vars(input_value: dict[str, Json]) -> Json:
    steps = input_value["steps"]
    if not isinstance(steps, list):
        raise ValueError("steps must be a list")
    extra_value = input_value.get("extra")
    if not isinstance(extra_value, list) or not all(isinstance(item, str) for item in extra_value):
        raise ValueError("extra must be a list of strings")
    extra = set(cast(list[str], extra_value))
    with (
        restored_globals(
            optional_text(input_value, "salt_key"),
            optional_text(input_value, "master_key"),
            input_value.get("general_settings"),
            text(input_value, "nonce_hex"),
        ),
        silenced_proxy_logger(),
    ):
        results: list[Json] = []
        for step in steps:
            if not isinstance(step, dict):
                raise ValueError("each step must be an object")
            results.append(call_callback(step, extra))
        return results


def run_master_key_match(input_value: dict[str, Json]) -> Json:
    presented = optional_text(input_value, "presented")
    master_key = optional_text(input_value, "master_key")
    return spend_tracking_utils._is_master_key(presented, master_key)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]


def run(input_value: Json) -> Json:
    if not isinstance(input_value, dict):
        raise ValueError("input must be an object")
    match text(input_value, "op"):
        case "app_crypto":
            return run_app_crypto(input_value)
        case "callback_vars":
            return run_callback_vars(input_value)
        case "master_key_match":
            return run_master_key_match(input_value)
        case other:
            raise ValueError(f"unknown stored_cipher op: {other}")
