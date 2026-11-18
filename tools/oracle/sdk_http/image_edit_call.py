from typing import ClassVar

from not_built import Unbuilt


class ImageEditCallRequest(Unbuilt):
    task: ClassVar[str] = "B999"

    image_edit_call: None
