import asyncio
import logging
import time
from typing import Final

from pydantic import JsonValue

from betterlite._integrations import custom_logger_types
from betterlite._integrations.custom_logger import CustomLogger

LOGGER: Final = logging.getLogger("LiteLLM")
SEND_ERROR: Final = "CustomLogger: async_send_batch raised; preserving %s events in queue for retry"
OVERFLOW: Final = "CustomLogger: log queue exceeded max_queue_size=%s; dropped %s oldest events."


class CustomBatchLogger(CustomLogger):
    # ast-grep-ignore: no-long-names
    preserve_events_added_during_flush = False
    DEFAULT_MAX_QUEUE_SIZE = 50_000

    def __init__(
        self,
        flush_lock: asyncio.Lock | None = None,
        batch_size: int | None = None,
        flush_interval: float | None = None,
        max_queue_size: int | None = None,
        **kwargs: bool,
    ) -> None:
        self.log_queue: list[JsonValue] = []
        self.flush_interval: float = (
            custom_logger_types.DEFAULT_FLUSH_INTERVAL_SECONDS
            if flush_interval is None
            else flush_interval
        )
        self.batch_size: int = (
            custom_logger_types.DEFAULT_BATCH_SIZE if batch_size is None else batch_size
        )
        self.last_flush_time: float = time.time()
        self.flush_lock: asyncio.Lock | None = flush_lock
        self.max_queue_size: int = (
            self.DEFAULT_MAX_QUEUE_SIZE if max_queue_size is None else max_queue_size
        )
        super().__init__(**kwargs)

    async def periodic_flush(self) -> None:
        while True:
            await asyncio.sleep(self.flush_interval)
            LOGGER.debug("CustomLogger periodic flush after %s seconds", self.flush_interval)
            await self.flush_queue()

    async def flush_queue(self) -> None:
        flush_lock = self.flush_lock
        if flush_lock is None:
            return
        async with flush_lock:
            if self.log_queue:
                log_queue_length = len(self.log_queue)
                LOGGER.debug("CustomLogger: Flushing batch of %s events", len(self.log_queue))
                try:
                    await self.async_send_batch()
                except Exception:
                    LOGGER.exception(SEND_ERROR, log_queue_length)
                    overflow = len(self.log_queue) - self.max_queue_size
                    if overflow > 0:
                        del self.log_queue[:overflow]
                        LOGGER.warning(OVERFLOW, self.max_queue_size, overflow)
                    return
                if self.preserve_events_added_during_flush:
                    del self.log_queue[:log_queue_length]
                else:
                    self.log_queue.clear()
                self.last_flush_time = time.time()

    async def async_send_batch(self, *args: JsonValue, **kwargs: JsonValue) -> None: ...
