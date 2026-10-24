import logging
import os
import sys
from collections.abc import Callable, Generator
from contextlib import contextmanager
from types import ModuleType
from typing import TypeAlias, cast, override

import nacl.secret
from litellm.proxy.common_utils import encrypt_decrypt_utils as codec

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
DOCS_LINK = "https://docs.litellm.ai/docs/proxy/prod#5-set-litellm-salt-key"
FIELDS = {"salt_key", "master_key", "general_settings", "nonce_hex", "steps"}

# LiteLLM types these helpers for str only or not at all, so strict mode needs the JSON shape
encrypt_value = cast(Callable[[Json, str | None], Json], codec.encrypt_value_helper)
decrypt_value = cast(Callable[[Json, str, str, bool], Json], codec.decrypt_value_helper)
decode_map = cast(Callable[..., Json], codec.decode_secret_map)
encrypt_map = cast(Callable[[Json, str | None], str], codec.encrypt_secret_map)


class LogCapture(logging.Filter):
    def __init__(self, field: str) -> None:
        super().__init__()
        self.field = field
        self.records: list[Json] = []

    @override
    def filter(self, record: logging.LogRecord) -> bool:
        message = record.getMessage()
        self.records.append({
            "level": record.levelname.lower(),
            "names_field": bool(self.field) and self.field in message,
            "salt_advice": DOCS_LINK in message,
        })
        return False


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


def call(step: dict[str, Json]) -> Json:
    match text(step, "op"):
        case "encrypt_value":
            result = encrypt_value(step.get("value"), optional_text(step, "new_key"))
        case "decrypt_value":
            keep = step.get("keep_original") is True
            report = text(step, "report")
            result = decrypt_value(step.get("value"), text(step, "field"), report, keep)
        case "decrypt_with":
            signing_key = text(step, "signing_key")
            result = codec.decrypt_if_encrypted_with(text(step, "value"), signing_key)
        case "encrypt_bearer":
            result = codec.encrypt_bearer_token(text(step, "value"), text(step, "prefix"))
        case "decrypt_bearer":
            result = codec.decrypt_bearer_token(text(step, "value"), text(step, "prefix"))
        case "encrypt_map":
            result = encrypt_map(step.get("value"), optional_text(step, "new_key"))
        case "decode_map":
            result = decode_map(step.get("value"), key=text(step, "field"))
        case other:
            raise ValueError(f"unknown secret_codec op: {other}")
    return result


def run_step(step: Json) -> Json:
    if not isinstance(step, dict):
        raise ValueError("each step must be an object")
    capture = LogCapture(optional_text(step, "field") or "")
    logger = logging.getLogger("LiteLLM Proxy")
    logger.filters.insert(0, capture)
    try:
        result: dict[str, Json] = {"value": call(step)}
    except (ValueError, AttributeError, TypeError, codec.SecretMapDecodeError) as error:
        result = {"error": {"class": type(error).__name__, "message": str(error)}}
    finally:
        logger.removeFilter(capture)
    result["logs"] = capture.records
    return result


@contextmanager
def restored_globals(input_value: dict[str, Json]) -> Generator[None]:
    settings = input_value.get("general_settings")
    fixed = FixedRandom(bytes.fromhex(text(input_value, "nonce_hex")))
    # nacl.secret binds random at import, so the box only sees a replacement in its own namespace
    box_namespace = cast(dict[str, object], vars(nacl.secret))
    saved_box_random = box_namespace["random"]
    saved_urandom = os.urandom
    saved_salt = os.environ.pop("LITELLM_SALT_KEY", None)
    logger = logging.getLogger("LiteLLM Proxy")
    saved_level = logger.level
    salt = optional_text(input_value, "salt_key")
    if salt is not None:
        os.environ["LITELLM_SALT_KEY"] = salt
    # the codec reads only these two globals, and the real module adds seconds to every helper call
    proxy_server = ModuleType("litellm.proxy.proxy_server")
    proxy_namespace = cast(dict[str, Json], vars(proxy_server))
    proxy_namespace["master_key"] = optional_text(input_value, "master_key")
    if isinstance(settings, dict):
        # a missing global makes LiteLLM's import fail, its "settings unavailable" path
        proxy_namespace["general_settings"] = settings
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


def run(input_value: dict[str, Json]) -> Json:
    if set(input_value) != FIELDS:
        raise ValueError(f"input fields must be {sorted(FIELDS)}")
    steps = input_value["steps"]
    if not isinstance(steps, list):
        raise ValueError("steps must be a list")
    with restored_globals(input_value):
        return [run_step(step) for step in steps]
