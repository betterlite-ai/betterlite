import asyncio
import logging
import os
import sys
from types import ModuleType
from typing import Final, Literal, TypeAlias, TypedDict

from litellm.proxy._experimental.mcp_server.outbound_credentials.sso_assertion_store import (
    rotate_sso_identity_assertions_master_key,
)
from litellm.proxy.common_utils.encrypt_decrypt_utils import (
    decrypt_if_encrypted_with,
    encrypt_secret_map,
)
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

from b10010.prisma_double import AssertionRow, AssertionTable, FakePrismaClient, ServerTable

SALT_ENV: Final = "LITELLM_SALT_KEY"
PROXY_LOGGER: Final = "LiteLLM Proxy"
PROXY_SERVER_NAME: Final = "litellm.proxy.proxy_server"
ROTATION_PREFIX: Final = "rotate_sso_identity_assertions_master_key:"
# the Rust side of this operation seeds and rotates the same two keys
SALT_KEY: Final = "b10010-salt-key"
NEW_KEY: Final = "b10010-new-key"
TEXT: Final[TypeAdapter[str]] = TypeAdapter(str)


class ProxyServerStub(ModuleType):
    def __init__(self) -> None:
        super().__init__(PROXY_SERVER_NAME)
        self.master_key: str | None = None
        self.general_settings: dict[str, JsonValue] = {}


class Capture(logging.Filter):
    def __init__(self) -> None:
        super().__init__()
        self.messages: list[dict[str, str]] = []

    @override
    def filter(self, record: logging.LogRecord) -> bool:
        message = record.getMessage()
        if message.startswith(ROTATION_PREFIX):
            self.messages.append({"level": record.levelname, "message": message})
        return False


class PlainRow(TypedDict):
    kind: Literal["plain"]
    user_id: str
    payload: str


class RawRow(TypedDict):
    kind: Literal["raw"]
    user_id: str
    text: str


RowSeed: TypeAlias = PlainRow | RawRow


class Input(TypedDict):
    rows: list[RowSeed]


class RotatedRow(TypedDict):
    kind: Literal["rotated"]
    user_id: str
    plaintext: str


class SkippedRow(TypedDict):
    kind: Literal["skipped"]
    user_id: str
    text: str


class Answer(TypedDict):
    rows: list[RotatedRow | SkippedRow]
    logs: list[dict[str, str]]
    updates: int


def sealed_payload(payload: str) -> str:
    # encrypt_secret_map JSON-encodes its output; the stored column holds the ciphertext itself
    sealed = encrypt_secret_map({"assertion": payload}, new_encryption_key=SALT_KEY)
    return TEXT.validate_json(sealed)


def seeded_row(seed: RowSeed) -> AssertionRow:
    if seed["kind"] == "plain":
        return AssertionRow(seed["user_id"], sealed_payload(seed["payload"]))
    return AssertionRow(seed["user_id"], seed["text"])


def row_answer(row: AssertionRow) -> RotatedRow | SkippedRow:
    plaintext = decrypt_if_encrypted_with(row.assertion_b64, NEW_KEY)
    if plaintext is None:
        return SkippedRow(kind="skipped", user_id=row.user_id, text=row.assertion_b64)
    return RotatedRow(kind="rotated", user_id=row.user_id, plaintext=plaintext)


def rotate(table: AssertionTable, capture: Capture) -> None:
    logger = logging.getLogger(PROXY_LOGGER)
    # the proxy logger's handlers write INFO to stdout, the channel the helper answer travels on
    for handler in list(logger.handlers):
        logger.removeHandler(handler)
    logger.propagate = False
    logger.setLevel(logging.INFO)
    logger.filters.insert(0, capture)
    try:
        asyncio.run(
            rotate_sso_identity_assertions_master_key(
                prisma_client=FakePrismaClient(table, ServerTable([])),
                new_master_key=NEW_KEY,
            )
        )
    finally:
        logger.filters.remove(capture)


def run(input_value: Input) -> Answer:
    os.environ[SALT_ENV] = SALT_KEY
    sys.modules[PROXY_SERVER_NAME] = ProxyServerStub()
    table = AssertionTable([seeded_row(seed) for seed in input_value["rows"]])
    capture = Capture()
    rotate(table, capture)
    return {
        "rows": [row_answer(row) for row in table.rows],
        "logs": capture.messages,
        "updates": len(table.updates),
    }
