from typing import ClassVar

from not_built import Unbuilt


class AssistantThreadsRequest(Unbuilt):
    task: ClassVar[str] = "B1056"

    assistant_threads: None
