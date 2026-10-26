from typing import TypedDict


class Input(TypedDict):
    filename: str


def run(input_value: Input) -> str:
    filename = input_value["filename"]
    if "\x00" in filename:
        raise ValueError("Filename contains null byte")
    basename = filename.replace("\\", "/").rsplit("/", 1)[-1]
    if not basename or basename in (".", ".."):
        raise ValueError("Empty or unsafe filename")
    return basename
