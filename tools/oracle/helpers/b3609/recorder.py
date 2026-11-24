import datetime as dt
from collections.abc import Coroutine
from typing import Final

from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj
from litellm.types.passthrough_endpoints.pass_through_endpoints import EndpointType
from pydantic import JsonValue

from .model import FailureRecord, ScriptedReadError, UsageAndCost


class Recorder:
    def __init__(self) -> None:
        self.reset(poller=False, run_id=None)

    def reset(self, *, poller: bool, run_id: str | None) -> None:
        self.poller = poller
        self.run_id = run_id
        self.jobs: list[Coroutine[None, None, None]] = []
        self.routes: list[list[bytes]] = []
        self.started: list[tuple[str, dt.datetime]] = []
        self.scans = 0
        self.failures: list[FailureRecord] = []
        self.partial: UsageAndCost | None = None
        self.error: ScriptedReadError | None = None
        self.disconnected = False
        self.params: dict[str, JsonValue] = {
            "metadata": {"user_api_key_budget_reservation": {"id": "reservation-1"}}
        }

    def __call__(self, async_coroutine: Coroutine[None, None, None]) -> None:
        self.jobs.append(async_coroutine)

    async def route(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/streaming_handler.py#L41-L53
        self,
        *,
        litellm_logging_obj: LiteLLMLoggingObj,
        url_route: str,
        request_body: dict[str, JsonValue],
        endpoint_type: EndpointType,
        start_time: dt.datetime,
        raw_bytes: list[bytes],
        end_time: dt.datetime,
        **kwargs: JsonValue,
    ) -> None:
        _ = (
            litellm_logging_obj,
            url_route,
            request_body,
            endpoint_type,
            start_time,
            end_time,
            kwargs,
        )
        self.routes.append(list(raw_bytes))


RECORDER: Final = Recorder()
