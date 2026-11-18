from typing import ClassVar

from not_built import Unbuilt


class DecisionsSendRequest(Unbuilt):
    task: ClassVar[str] = "B3514"

    decisions_send: None
