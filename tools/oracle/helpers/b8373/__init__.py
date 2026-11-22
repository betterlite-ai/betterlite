import asyncio
import logging
from collections.abc import Mapping
from typing import TypedDict

from litellm.caching.redis_cache import RedisCache
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.proxy_server import ProxyStartupEvent
from litellm.proxy.utils import PrismaClient
from pydantic import JsonValue
from typing_extensions import override

PROXY_LOGGER = "LiteLLM Proxy"


class Input(TypedDict):
    operation: str
    use_redis_transaction_buffer: bool | str
    redis_configured: bool
    max_budget: float
    database_configured: bool
    enabled: bool
    row: JsonValue


class Capture(logging.Filter):
    def __init__(self) -> None:
        super().__init__()
        self.messages: list[str] = []

    @override
    def filter(self, record: logging.LogRecord) -> bool:
        self.messages.append(record.getMessage())
        return False


class ConfigRow:
    def __init__(self, param_value: JsonValue) -> None:
        self.param_value = param_value


class ConfigTable:
    def __init__(self, row: ConfigRow | None) -> None:
        self.row = row
        self.queries: list[dict[str, str]] = []

    async def find_unique(self, *, where: Mapping[str, str]) -> ConfigRow | None:
        self.queries.append(dict(where))
        return self.row


class FakeDb(PrismaWrapper):
    def __init__(self, table: ConfigTable) -> None:
        self.litellm_config = table


class FakePrismaClient(PrismaClient):
    def __init__(self, table: ConfigTable) -> None:
        self.db = FakeDb(table)


class FakeRedisCache(RedisCache):
    def __init__(self) -> None:
        pass


def install_capture() -> Capture:
    logger = logging.getLogger(PROXY_LOGGER)
    for handler in list(logger.handlers):
        logger.removeHandler(handler)
    logger.propagate = False
    logger.setLevel(logging.INFO)
    capture = Capture()
    logger.filters.insert(0, capture)
    return capture


def warning_answer(capture: Capture) -> JsonValue:
    answer: JsonValue = {"warned": False, "message": None}
    if capture.messages:
        answer = {"warned": True, "message": capture.messages[0]}
    return answer


def transaction_buffer(input_value: Input) -> JsonValue:
    flag = input_value["use_redis_transaction_buffer"]
    cache: RedisCache | None = None
    if input_value["redis_configured"]:
        cache = FakeRedisCache()
    general_settings: dict[str, JsonValue] = {"use_redis_transaction_buffer": flag}
    answer: JsonValue = {"raised": False, "message": None}
    try:
        ProxyStartupEvent._validate_redis_transaction_buffer_config(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L10229-L10257
            general_settings=general_settings,
            redis_usage_cache=cache,
        )
    except ValueError as error:
        answer = {"raised": True, "message": str(error)}
    return answer


def budget_warning(input_value: Input) -> JsonValue:
    capture = install_capture()
    prisma_client = FakePrismaClient(ConfigTable(None)) if input_value["database_configured"] else None
    ProxyStartupEvent._warn_budget_without_db(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L10157-L10168
        max_budget=input_value["max_budget"], prisma_client=prisma_client
    )
    return warning_answer(capture)


def fail_closed_warning(input_value: Input) -> JsonValue:
    capture = install_capture()
    cache = FakeRedisCache() if input_value["redis_configured"] else None
    ProxyStartupEvent._warn_fail_closed_rate_limits_without_redis(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L10170-L10179
        fail_closed_rate_limit_enforcement=input_value["enabled"], redis_usage_cache=cache
    )
    return warning_answer(capture)


def baseline_row(input_value: Input) -> JsonValue:
    table = ConfigTable(ConfigRow(input_value["row"]))
    client = FakePrismaClient(table)
    baselines = asyncio.run(
        ProxyStartupEvent._load_heuristic_v1_tuning_baselines(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L10495-L10540
            client, ()
        )
    )
    if baselines is None:
        return {}
    answer: dict[str, JsonValue] = {}
    for identity, fingerprint in baselines.items():
        text: JsonValue = fingerprint
        answer[identity] = text
    return answer


def run(input_value: Input) -> JsonValue:
    operation = input_value["operation"]
    if operation == "transaction_buffer":
        return transaction_buffer(input_value)
    if operation == "budget_warning":
        return budget_warning(input_value)
    if operation == "fail_closed_warning":
        return fail_closed_warning(input_value)
    if operation == "baseline_row":
        return baseline_row(input_value)
    raise ValueError(f"unknown startup-check operation: {operation}")
