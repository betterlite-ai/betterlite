from typing import ClassVar

from not_built import Unbuilt


class FileProviderWriteRequest(Unbuilt):
    task: ClassVar[str] = "B1028"

    file_provider_write: None
