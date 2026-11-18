from typing import ClassVar

from not_built import Unbuilt


class AssistantDispatchRequest(Unbuilt):
    task: ClassVar[str] = "B2489"

    assistant_dispatch: None
