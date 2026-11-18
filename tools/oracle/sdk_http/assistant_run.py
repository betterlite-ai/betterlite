from typing import ClassVar

from not_built import Unbuilt


class AssistantRunRequest(Unbuilt):
    task: ClassVar[str] = "B2488"

    assistant_run: None
