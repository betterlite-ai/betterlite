from typing import Literal, TypedDict


class Input(TypedDict):
    function: Literal["mime_extension_table", "mime_lookup_edges", "gemini_file_acceptance"]
    extensions: list[str]
    mimes: list[str]
    file_types: list[str]


EXTENSIONS: tuple[tuple[str, tuple[str, ...]], ...] = (
    ("AAC", ("aac",)),
    ("CSV", ("csv",)),
    ("DOC", ("doc",)),
    ("DOCX", ("docx",)),
    ("FLAC", ("flac",)),
    ("FLV", ("flv",)),
    ("GIF", ("gif",)),
    ("GOOGLE_DOC", ("gdoc",)),
    ("GOOGLE_DRAWINGS", ("gdraw",)),
    ("GOOGLE_SHEETS", ("gsheet",)),
    ("GOOGLE_SLIDES", ("gslides",)),
    ("HEIC", ("heic",)),
    ("HEIF", ("heif",)),
    ("HTML", ("html", "htm")),
    ("JPEG", ("jpeg", "jpg")),
    ("JSON", ("json",)),
    ("M4A", ("m4a",)),
    ("M4V", ("m4v",)),
    ("MOV", ("mov",)),
    ("MP3", ("mp3",)),
    ("MP4", ("mp4",)),
    ("MPEG", ("mpeg",)),
    ("MPEGPS", ("mpegps",)),
    ("MPG", ("mpg",)),
    ("MPA", ("mpa",)),
    ("MPGA", ("mpga",)),
    ("OGG", ("ogg",)),
    ("OPUS", ("opus",)),
    ("PDF", ("pdf",)),
    ("PCM", ("pcm",)),
    ("PNG", ("png",)),
    ("PPT", ("ppt",)),
    ("PPTX", ("pptx",)),
    ("RTF", ("rtf",)),
    ("3GPP", ("3gpp",)),
    ("TXT", ("txt",)),
    ("WAV", ("wav",)),
    ("WEBM", ("webm",)),
    ("WEBP", ("webp",)),
    ("WMV", ("wmv",)),
    ("XLS", ("xls",)),
    ("XLSX", ("xlsx",)),
)
MIMES: dict[str, str] = {
    "AAC": "audio/aac",
    "CSV": "text/csv",
    "DOC": "application/msword",
    "DOCX": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "FLAC": "audio/flac",
    "FLV": "video/x-flv",
    "GIF": "image/gif",
    "GOOGLE_DOC": "application/vnd.google-apps.document",
    "GOOGLE_DRAWINGS": "application/vnd.google-apps.drawing",
    "GOOGLE_SHEETS": "application/vnd.google-apps.spreadsheet",
    "GOOGLE_SLIDES": "application/vnd.google-apps.presentation",
    "HEIC": "image/heic",
    "HEIF": "image/heif",
    "HTML": "text/html",
    "JPEG": "image/jpeg",
    "JSON": "application/json",
    "M4A": "audio/x-m4a",
    "M4V": "video/x-m4v",
    "MOV": "video/quicktime",
    "MP3": "audio/mpeg",
    "MP4": "video/mp4",
    "MPEG": "video/mpeg",
    "MPEGPS": "video/mpegps",
    "MPG": "video/mpg",
    "MPA": "audio/m4a",
    "MPGA": "audio/mpga",
    "OGG": "audio/ogg",
    "OPUS": "audio/opus",
    "PDF": "application/pdf",
    "PCM": "audio/pcm",
    "PNG": "image/png",
    "PPT": "application/vnd.ms-powerpoint",
    "PPTX": "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    "RTF": "application/rtf",
    "3GPP": "video/3gpp",
    "TXT": "text/plain",
    "WAV": "audio/wav",
    "WEBM": "video/webm",
    "WEBP": "image/webp",
    "WMV": "video/wmv",
    "XLS": "application/vnd.ms-excel",
    "XLSX": "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
}
ACCEPTED = frozenset(
    (
        "PNG",
        "JPEG",
        "WEBP",
        "AAC",
        "FLAC",
        "MP3",
        "MPA",
        "MPEG",
        "MPGA",
        "OPUS",
        "PCM",
        "WAV",
        "WEBM",
        "FLV",
        "MOV",
        "MPEGPS",
        "MPG",
        "MP4",
        "WMV",
        "3GPP",
        "PDF",
        "TXT",
    )
)


def extension_type(extension: str) -> str:
    for file_type, values in EXTENSIONS:
        if extension.lower() in values:
            return file_type
    raise ValueError(f"Unknown file type for extension: {extension}")


def mime_extension(mime: str) -> str:
    for file_type, value in MIMES.items():
        if value.lower() == mime.lower():
            return next(values[0] for name, values in EXTENSIONS if name == file_type)
    raise ValueError(f"Unknown extension for mime type: {mime}")


def run(input_value: Input) -> dict[str, object] | list[object]:
    if input_value["function"] == "mime_extension_table":
        return [
            {"extension": extension, "mime": MIMES[extension_type(extension)]}
            for extension in input_value["extensions"]
        ]
    if input_value["function"] == "mime_lookup_edges":
        return {
            "extensions": [
                MIMES[extension_type(extension)] for extension in input_value["extensions"]
            ],
            "mimes": [mime_extension(mime) for mime in input_value["mimes"]],
        }
    return [file_type in ACCEPTED for file_type in input_value["file_types"]]
