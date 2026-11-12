from importlib import metadata

try:
    version = metadata.version("betterlite")
except metadata.PackageNotFoundError:
    version = "unknown"
