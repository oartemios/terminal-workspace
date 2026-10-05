"""Validate tag artifacts and publish a complete draft; no third-party Python deps."""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import tarfile
import tempfile
from urllib.error import HTTPError
from urllib.parse import quote
from urllib.request import Request, urlopen

from version import build_metadata

TARGETS = (
    "aarch64-apple-darwin", "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin", "x86_64-unknown-linux-gnu",
)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def release_notes(changelog, version):
    sections = re.split(r"(?m)^## ", changelog)
    matches = [section.partition("\n")[2].strip() for section in sections[1:]
               if section.splitlines()[0].split(" — ")[0].strip() == version]
    if len(matches) != 1 or not matches[0]:
        raise ValueError(f"Expected one nonempty CHANGELOG section for {version}")
    return matches[0] + "\n"


def verify_artifacts(directory, version, commit):
    files = [path for path in directory.rglob("*") if path.is_file()]
    expected = {f"terminal-workspace-{version}-{target}.tar.gz{suffix}"
                for target in TARGETS for suffix in ("", ".sha256")}
    if len(files) != 8 or {path.name for path in files} != expected:
        raise ValueError("Expected exactly four platform archives and four checksums")
    by_name = {path.name: path for path in files}
    for target in TARGETS:
        prefix = f"terminal-workspace-{version}-{target}"
        archive = by_name[f"{prefix}.tar.gz"]
        checksum = by_name[f"{prefix}.tar.gz.sha256"].read_text().split()
        if checksum != [digest(archive), archive.name]:
            raise ValueError(f"SHA-256 mismatch: {archive.name}")
        with tarfile.open(archive, "r:gz") as bundle:
            members = bundle.getmembers()
            names = [member.name.rstrip("/") for member in members]
            if len(names) != len(set(names)):
                raise ValueError(f"Duplicate archive members: {archive.name}")
            for member in members:
                path = PurePosixPath(member.name)
                if (path.is_absolute() or ".." in path.parts or not path.parts
                        or path.parts[0] != prefix or not (member.isfile() or member.isdir())):
                    raise ValueError(f"Unsafe archive member: {member.name}")
            for name, value in {"VERSION": version, "BUILD_VERSION": version,
                                "COMMIT": commit, "TARGET": target}.items():
                member = bundle.getmember(f"{prefix}/{name}")
                if not member.isfile() or bundle.extractfile(member).read().decode().strip() != value:
                    raise ValueError(f"Incorrect {name}: {archive.name}")
            binary = bundle.getmember(f"{prefix}/tw")
            if not binary.isfile() or not binary.mode & 0o111:
                raise ValueError(f"Missing executable tw: {archive.name}")
    return sorted(files)


class GitHub:
    def __init__(self, repo, token):
        if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repo):
            raise ValueError("Invalid GITHUB_REPOSITORY")
        self.repo = repo
        self.token = token

    def api(self, path, data=None, method=None, missing_ok=False):
        request = Request(f"https://api.github.com/repos/{self.repo}/{path}",
                          data=json.dumps(data).encode() if data is not None else None,
                          method=method, headers={
                              "Authorization": f"Bearer {self.token}",
                              "Accept": "application/vnd.github+json",
                              "Content-Type": "application/json",
                              "X-GitHub-Api-Version": "2022-11-28",
                          })
        try:
            with urlopen(request, timeout=60) as response:
                return json.load(response)
        except HTTPError as error:
            if missing_ok and error.code == 404:
                return None
            raise

    def tag_commit(self, tag):
        obj = self.api(f"git/ref/tags/{quote(tag, safe='')}")["object"]
        for _ in range(10):
            if obj["type"] == "commit":
                return obj["sha"]
            if obj["type"] != "tag":
                break
            obj = self.api(f"git/tags/{obj['sha']}")["object"]
        raise ValueError("Release tag does not resolve to a commit")

    def transfer(self, operation, tag, files_or_directory):
        env = dict(os.environ, GH_TOKEN=self.token)
        args = ["gh", "release", operation, tag, "--repo", self.repo]
        if operation == "upload":
            args += [str(path) for path in files_or_directory] + ["--clobber"]
        else:
            args += ["--dir", str(files_or_directory)]
        subprocess.run(args, check=True, env=env, timeout=300)


def publish(client, tag, commit, notes, files):
    if client.tag_commit(tag) != commit:
        raise ValueError("Remote release tag no longer matches the build commit")
    release = client.api(f"releases/tags/{quote(tag, safe='')}", missing_ok=True)
    if release is not None and not release["draft"]:
        print(f"Release {tag} already published; leaving it unchanged")
        return
    details = {"tag_name": tag, "target_commitish": commit,
               "name": f"Terminal Workspace {tag[1:]}", "body": notes,
               "draft": True, "prerelease": "-" in tag}
    if release is None:
        release = client.api("releases", details)
    else:
        release = client.api(f"releases/{release['id']}", details, method="PATCH")
    client.transfer("upload", tag, files)
    release = client.api(f"releases/{release['id']}")
    expected = {path.name: path for path in files}
    assets = release["assets"]
    if (not release["draft"] or len(assets) != len(expected)
            or {asset["name"] for asset in assets} != set(expected)
            or any(asset["state"] != "uploaded" or asset["size"] != expected[asset["name"]].stat().st_size
                   for asset in assets)):
        raise ValueError("Draft assets are incomplete or contain unexpected files")
    # Check bytes actually uploaded, including checksums, before making the release public.
    with tempfile.TemporaryDirectory() as temporary:
        client.transfer("download", tag, Path(temporary))
        for name, path in expected.items():
            if digest(Path(temporary) / name) != digest(path):
                raise ValueError(f"Uploaded asset differs: {name}")
    if client.tag_commit(tag) != commit:
        raise ValueError("Remote release tag changed during upload")
    published = client.api(f"releases/{release['id']}", {"draft": False}, method="PATCH")
    if published["draft"]:
        raise ValueError("Release is still a draft")
    print(published["html_url"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts", type=Path, required=True)
    parser.add_argument("--check-only", action="store_true")
    args = parser.parse_args()
    ref = os.environ.get("GITHUB_REF", "")
    if not ref.startswith("refs/tags/v"):
        raise ValueError("Publication requires a release tag ref")
    metadata = json.loads(subprocess.check_output(["python3", "scripts/version.py"], text=True))
    build_metadata(metadata["version"], metadata["commit"], ref)
    if metadata["commit"] != os.environ.get("GITHUB_SHA"):
        raise ValueError("Checkout commit does not match GITHUB_SHA")
    files = verify_artifacts(args.artifacts, metadata["version"], metadata["commit"])
    notes = release_notes(Path("CHANGELOG.md").read_text(), metadata["version"])
    if args.check_only:
        print("Validated four archives, checksums, metadata and release notes")
        return
    publish(GitHub(os.environ["GITHUB_REPOSITORY"], os.environ["GH_TOKEN"]),
            ref.removeprefix("refs/tags/"), metadata["commit"], notes, files)


if __name__ == "__main__":
    main()
