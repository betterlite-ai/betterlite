from typing import ClassVar

from not_built import Unbuilt


class RealtimeHttpCallsRequest(Unbuilt):
    task: ClassVar[str] = "B1010"

    realtime_http_calls: None
