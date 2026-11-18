import asyncio
from logging import Handler, LogRecord, getLogger
from typing import Final, Literal, NotRequired, TypedDict, TypeAlias

from litellm.caching.in_memory_cache import InMemoryCache
from litellm.caching.redis_cache import RedisCache
from litellm.proxy import proxy_server
from litellm.proxy.common_utils.auth_cache_invalidation_pubsub import (
    AuthCacheInvalidationSubscriber,
    evict_and_broadcast,
    publish_auth_cache_invalidation,
)
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from pydantic import JsonValue, TypeAdapter
from redis.asyncio import Redis
from redis.asyncio.client import PubSub
from redis.typing import ChannelT, EncodableT

CLOCK_NOW: Final[float] = 1000.0
DEFAULT_TTL: Final[float] = 600.0
MAX_PENDING: Final[int] = 1024
JSON_VALUES: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
PUBLICATIONS: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
TEXTS: Final[TypeAdapter[str]] = TypeAdapter(str)
NUMBERS: Final[TypeAdapter[float]] = TypeAdapter(float)
PublishMode: TypeAlias = Literal["accept", "fail", "block"]


class EvictCall(TypedDict):
    keys: list[str]
    fail_publish: NotRequired[bool]


class StepCall(TypedDict):
    kind: Literal["block", "fail", "message"]
    message: NotRequired[str]


class SubscriberCall(TypedDict):
    sessions: list[list[StepCall]]
    probe_keys: NotRequired[list[str]]


class Input(TypedDict):
    evict: NotRequired[EvictCall]
    capacity: NotRequired[bool]
    subscriber: NotRequired[SubscriberCall]


class Journal:
    def __init__(self) -> None:
        self.events: list[str] = []
        self.publications: list[JsonValue] = []
        self.warnings: list[JsonValue] = []
        self.caches: dict[str, JsonValue] = {}
        self.blocker = asyncio.Event()


class FixedClock:
    def __init__(self) -> None:
        self.now = CLOCK_NOW

    def __call__(self) -> float:
        return self.now


class WarningHandler(Handler):
    def __init__(self, journal: Journal) -> None:
        super().__init__()
        self.journal = journal

    def emit(self, record: LogRecord) -> None:
        self.journal.warnings.append(
            {"level": record.levelname.lower(), "message": record.getMessage()}
        )


class RecordingCache(InMemoryCache):
    def __init__(self, journal: Journal) -> None:
        super().__init__(clock=FixedClock())
        self.journal = journal

    def delete_cache(self, key: str) -> None:
        self.journal.events.append(f"delete:{key}")
        super().delete_cache(key)


class FakePubSub(PubSub):
    def __init__(self, journal: Journal, steps: list[StepCall]) -> None:
        self.journal = journal
        self.steps = steps
        self.subscribed = False

    def __del__(self) -> None:
        pass

    async def subscribe(self, *args: object, **kwargs: object) -> None:
        del args, kwargs
        self.journal.events.append("subscribe")
        self.subscribed = True
        await asyncio.sleep(0)

    async def get_message(self, **kwargs: object) -> dict[str, JsonValue] | None:
        timeout = kwargs["timeout"] if "timeout" in kwargs else 0.0
        interval = 0.0 if timeout is None else NUMBERS.validate_python(timeout)
        self.journal.events.append(f"poll:{int(interval * 1000)}")
        if not self.steps:
            await self.journal.blocker.wait()
            return None
        step = self.steps.pop(0)
        if step["kind"] == "block":
            await self.journal.blocker.wait()
            return None
        if step["kind"] == "fail":
            raise RuntimeError("scripted consume failed")
        self.journal.events.append("message")
        return {"data": step["message"] if "message" in step else ""}

    async def aclose(self) -> None:
        if self.subscribed:
            self.journal.events.append("close")
        await asyncio.sleep(0)


class FakeClient(Redis):
    def __init__(self, journal: Journal, mode: PublishMode, sessions: list[list[StepCall]]) -> None:
        self.journal = journal
        self.mode = mode
        self.sessions = sessions

    def pubsub(self, **kwargs: object) -> PubSub:
        del kwargs
        return FakePubSub(self.journal, self.sessions.pop(0) if self.sessions else [])

    async def publish(self, channel: ChannelT, message: EncodableT, **kwargs: object) -> int:
        del channel, kwargs
        self.journal.events.append("publish-started")
        if self.mode == "fail":
            self.journal.events.append("publish-failed")
            raise RuntimeError("scripted publish failed")
        if self.mode == "block":
            await self.journal.blocker.wait()
            return 0
        parsed = PUBLICATIONS.validate_json(TEXTS.validate_python(message))
        self.journal.publications.append(
            {
                "cache_key": parsed["cache_key"],
                "new_value": parsed["new_value"] if "new_value" in parsed else None,
                "ttl": parsed["ttl"] if "ttl" in parsed else None,
            }
        )
        self.journal.events.append("publish")
        return 0


class FakeRedisCache(RedisCache):
    def __init__(self, journal: Journal, mode: PublishMode, sessions: list[list[StepCall]]) -> None:
        self.journal = journal
        self.mode = mode
        self.sessions = sessions

    def init_pubsub_client(self) -> Redis:
        return FakeClient(self.journal, self.mode, self.sessions)


def json_value(journal: Journal) -> dict[str, JsonValue]:
    return {
        "events": journal.events,
        "publications": journal.publications,
        "warnings": journal.warnings,
        "caches": journal.caches,
    }


def planned_polls(sessions: list[list[StepCall]]) -> int:
    return sum(max(len(steps), 1) for steps in sessions)


def state(memory: InMemoryCache, key: str) -> JsonValue:
    return {
        "value": JSON_VALUES.validate_python(memory.cache_dict.get(key)),
        "expiry": JSON_VALUES.validate_python(memory.ttl_dict.get(key)),
    }


async def wait_for_polls(journal: Journal, expected: int) -> None:
    for _attempt in range(600):
        polls = sum(1 for event in journal.events if event.startswith("poll:"))
        if polls >= expected:
            return
        await asyncio.sleep(0.05)
    raise RuntimeError(f"the subscriber never reached {expected} polls")


def install(journal: Journal, mode: PublishMode, sessions: list[list[StepCall]]) -> FakeRedisCache:
    redis = FakeRedisCache(journal, mode, sessions)
    proxy_server.redis_usage_cache = redis
    return redis


async def evict_section(call: EvictCall, journal: Journal) -> None:
    mode: PublishMode = "fail" if "fail_publish" in call and call["fail_publish"] else "accept"
    install(journal, mode, [])
    cache = UserApiKeyCache(in_memory_cache=RecordingCache(journal), redis_cache=None)
    await evict_and_broadcast(call["keys"], cache)


async def capacity_section(journal: Journal) -> None:
    install(journal, "block", [])
    for index in range(MAX_PENDING + 1):
        await publish_auth_cache_invalidation(f"cap:{index}")


async def subscriber_section(call: SubscriberCall, journal: Journal) -> None:
    redis = install(journal, "accept", call["sessions"])
    ordinary = InMemoryCache(clock=FixedClock())
    key_objects = InMemoryCache(clock=FixedClock())
    additional = InMemoryCache(clock=FixedClock())
    probe_keys = call["probe_keys"] if "probe_keys" in call else []
    for memory, value in ((ordinary, 2.0), (key_objects, 1.0), (additional, 3.0)):
        for key in probe_keys:
            memory.set_cache(key, value)
    cache = UserApiKeyCache(
        in_memory_cache=ordinary,
        redis_cache=None,
        key_object_in_memory_cache=key_objects,
    )
    subscriber = AuthCacheInvalidationSubscriber(redis, cache, [additional])
    subscriber.start()
    await wait_for_polls(journal, planned_polls(call["sessions"]))
    await subscriber.stop()
    memories = {"ordinary": ordinary, "key_objects": key_objects, "additional:0": additional}
    for label, memory in memories.items():
        journal.caches[label] = {key: state(memory, key) for key in probe_keys}


async def dispatch(call: Input, journal: Journal) -> None:
    if "evict" in call:
        await evict_section(call["evict"], journal)
    if "capacity" in call and call["capacity"]:
        await capacity_section(journal)
    if "subscriber" in call:
        await subscriber_section(call["subscriber"], journal)


def run(input_value: Input) -> dict[str, JsonValue]:
    journal = Journal()
    handler = WarningHandler(journal)
    logger = getLogger("LiteLLM Proxy")
    logger.addHandler(handler)
    try:
        asyncio.run(dispatch(input_value, journal))
    finally:
        logger.removeHandler(handler)
    return json_value(journal)
