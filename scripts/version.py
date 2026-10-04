"""Read application version from Cargo; identify CI builds and validate release tags."""
import json
import os
from pathlib import Path
import re
import subprocess


def build_metadata(version, commit, ref):
    # SemVer prerelease is supported; build metadata is reserved for build identity.
    number = r"(?:0|[1-9][0-9]*)"
    identifier = r"(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)"
    if not re.fullmatch(rf"{number}\.{number}\.{number}(?:-{identifier}(?:\.{identifier})*)?", version):
        raise ValueError(f"Unsupported application SemVer: {version}")
    if ref.startswith("refs/tags/"):
        tag = ref.removeprefix("refs/tags/")
        if tag != f"v{version}":
            raise ValueError(f"Release tag {tag} must match Cargo version v{version}")
        return {"version": version, "build_version": version, "commit": commit}
    # SemVer build metadata distinguishes commits without implying a release.
    return {"version": version, "build_version": f"{version}+dev.{commit[:12]}", "commit": commit}


def main():
    metadata = json.loads(subprocess.check_output([
        "cargo", "metadata", "--no-deps", "--format-version", "1", "--locked", "--offline",
    ], text=True))
    package = next(p for p in metadata["packages"] if p["name"] == "terminal-workspace")
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    result = build_metadata(package["version"], commit, os.environ.get("GITHUB_REF", ""))
    output = os.environ.get("GITHUB_OUTPUT")
    if output:
        with Path(output).open("a") as stream:
            for key, value in result.items():
                stream.write(f"{key}={value}\n")
    print(json.dumps(result))


if __name__ == "__main__":
    main()
