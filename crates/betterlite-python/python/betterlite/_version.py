from importlib import metadata


def installed_version(distribution: str) -> str:
    try:
        found = metadata.version(distribution)
    except metadata.PackageNotFoundError:
        return "unknown"
    return found


version = installed_version("betterlite")
