"""Release validation/recovery checks, without network or real publication."""
import importlib.util
import io
from pathlib import Path
import shutil
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch
from urllib.error import HTTPError

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
spec = importlib.util.spec_from_file_location("release_script", Path(sys.path[0]) / "release.py")
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)
VERSION = "0.3.0"
COMMIT = "a" * 40


def archives(directory, **overrides):
    for target in release.TARGETS:
        prefix = f"terminal-workspace-{VERSION}-{target}"
        path = directory / f"{prefix}.tar.gz"
        values = dict(VERSION=VERSION, BUILD_VERSION=VERSION, COMMIT=COMMIT, TARGET=target, tw="binary")
        values.update(overrides)
        with tarfile.open(path, "w:gz") as bundle:
            for name, value in values.items():
                data = value.encode()
                info = tarfile.TarInfo(f"{prefix}/{name}")
                info.size = len(data)
                info.mode = 0o755 if name == "tw" else 0o644
                bundle.addfile(info, io.BytesIO(data))
        path.with_name(path.name + ".sha256").write_text(f"{release.digest(path)}  {path.name}\n")


class FakeGitHub:
    def __init__(self, files, draft=None):
        self.files = files
        self.release = None if draft is None else {"id": 1, "draft": draft}
        self.mutations = []
        self.uploaded = []
        self.fail_upload = False
        self.corrupt_download = False
        self.remote_commit = COMMIT

    def tag_commit(self, tag):
        return self.remote_commit

    def api(self, path, data=None, method=None, missing_ok=False):
        if data is not None:
            self.mutations.append(data.copy())
            self.release = dict(self.release or {}, **data, id=1, html_url="https://example/release")
        if self.release is None:
            return None
        return dict(self.release, assets=[dict(name=p.name, size=p.stat().st_size, state="uploaded")
                                         for p in self.uploaded])

    def transfer(self, operation, tag, paths):
        if operation == "upload":
            if self.fail_upload:
                self.uploaded = list(paths[:2])
                raise RuntimeError("upload interrupted")
            self.uploaded = list(paths)
        else:
            for path in self.uploaded:
                shutil.copyfile(path, paths / path.name)
            if self.corrupt_download:
                (paths / self.uploaded[0].name).write_bytes(b"corrupt")


class Release(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        archives(self.directory)
        self.files = release.verify_artifacts(self.directory, VERSION, COMMIT)

    def publish(self, client, tag="v0.3.0"):
        release.publish(client, tag, COMMIT, "Changes\n", self.files)

    def test_complete_platform_matrix(self):
        self.assertEqual(len(self.files), 8)

    def test_missing_extra_duplicate_or_corrupt_artifact(self):
        archive = self.files[0]
        original = archive.read_bytes()
        for mutation in (lambda: archive.unlink(),
                         lambda: (self.directory / "unexpected").write_text("extra"),
                         lambda: archive.write_bytes(b"corrupt")):
            with self.subTest(mutation=mutation):
                mutation()
                with self.assertRaises(ValueError):
                    release.verify_artifacts(self.directory, VERSION, COMMIT)
                archive.write_bytes(original)
                (self.directory / "unexpected").unlink(missing_ok=True)
        nested = self.directory / "duplicate"
        nested.mkdir()
        shutil.copyfile(archive, nested / archive.name)
        with self.assertRaises(ValueError):
            release.verify_artifacts(self.directory, VERSION, COMMIT)

    def test_wrong_metadata_even_with_valid_checksum(self):
        for field in ("COMMIT", "TARGET", "VERSION", "BUILD_VERSION"):
            with self.subTest(field=field):
                archives(self.directory, **{field: "wrong"})
                with self.assertRaises(ValueError):
                    release.verify_artifacts(self.directory, VERSION, COMMIT)

    def test_unsafe_archive_is_rejected(self):
        archives(self.directory, **{"../escape": "unsafe"})
        with self.assertRaises(ValueError):
            release.verify_artifacts(self.directory, VERSION, COMMIT)

    def test_notes_are_exact_section_and_nonempty(self):
        text = "# Changelog\n\n## Unreleased\n\n## 0.3.0 — 2026-10-05\n\nChanges\n### Details\nMore\n\n## 0.2.0 — old\nOld\n"
        self.assertEqual(release.release_notes(text, VERSION), "Changes\n### Details\nMore\n")
        for text in ("## Unreleased\nChanges", "## 0.3.0 — date\n\n", "## 0.3.0\nA\n## 0.3.0\nB"):
            with self.assertRaises(ValueError):
                release.release_notes(text, VERSION)

    def test_publish_and_prerelease(self):
        for tag in ("v0.3.0", "v0.3.0-rc.1"):
            client = FakeGitHub(self.files)
            self.publish(client, tag)
            self.assertFalse(client.release["draft"])
            self.assertEqual(client.release["prerelease"], "-" in tag)
            self.assertTrue(client.mutations[0]["draft"])

    def test_recover_partial_upload(self):
        client = FakeGitHub(self.files)
        client.fail_upload = True
        with self.assertRaises(RuntimeError):
            self.publish(client)
        self.assertTrue(client.release["draft"])
        client.fail_upload = False
        self.publish(client)
        self.assertFalse(client.release["draft"])
        self.assertEqual(len(client.uploaded), 8)

    def test_published_release_is_never_modified(self):
        client = FakeGitHub(self.files, draft=False)
        self.publish(client)
        self.assertEqual(client.mutations, [])
        self.assertEqual(client.uploaded, [])

    def test_remote_tag_mismatch_blocks_all_writes(self):
        client = FakeGitHub(self.files)
        client.remote_commit = "b" * 40
        with self.assertRaises(ValueError):
            self.publish(client)
        self.assertEqual(client.mutations, [])

    def test_tag_changed_during_upload_leaves_draft(self):
        client = FakeGitHub(self.files)
        transfer = client.transfer
        def changed(operation, tag, paths):
            transfer(operation, tag, paths)
            if operation == "upload":
                client.remote_commit = "b" * 40
        client.transfer = changed
        with self.assertRaises(ValueError):
            self.publish(client)
        self.assertTrue(client.release["draft"])

    def test_only_not_found_is_treated_as_missing_release(self):
        client = release.GitHub("owner/repo", "test-token")
        for code in (404, 403, 500):
            error = HTTPError("https://api.github.com", code, "error", {}, None)
            with patch.object(release, "urlopen", side_effect=error):
                if code == 404:
                    self.assertIsNone(client.api("releases/tags/v0.3.0", missing_ok=True))
                else:
                    with self.assertRaises(HTTPError):
                        client.api("releases/tags/v0.3.0", missing_ok=True)

    def test_annotated_and_lightweight_remote_tags(self):
        client = release.GitHub("owner/repo", "test-token")
        for responses in ([{"object": {"type": "commit", "sha": COMMIT}}],
                          [{"object": {"type": "tag", "sha": "b" * 40}},
                           {"object": {"type": "commit", "sha": COMMIT}}]):
            with patch.object(client, "api", side_effect=responses):
                self.assertEqual(client.tag_commit("v0.3.0"), COMMIT)

    def test_non_tag_ref_is_rejected_before_any_transfer(self):
        with patch.dict(release.os.environ, {"GITHUB_REF": "refs/heads/main"}), \
                patch.object(sys, "argv", ["release.py", "--artifacts", str(self.directory)]), \
                patch.object(release.subprocess, "check_output") as subprocess_call:
            with self.assertRaises(ValueError):
                release.main()
            subprocess_call.assert_not_called()

    def test_uploaded_bytes_must_match_before_publication(self):
        client = FakeGitHub(self.files)
        client.corrupt_download = True
        with self.assertRaises(ValueError):
            self.publish(client)
        self.assertTrue(client.release["draft"])

    def test_unexpected_assets_block_publication(self):
        client = FakeGitHub(self.files, draft=True)
        transfer = client.transfer
        extra = self.directory / "extra"
        extra.write_text("extra")
        def with_extra(operation, tag, paths):
            transfer(operation, tag, paths)
            if operation == "upload":
                client.uploaded.append(extra)
        client.transfer = with_extra
        with self.assertRaises(ValueError):
            self.publish(client)
        self.assertTrue(client.release["draft"])


if __name__ == "__main__":
    unittest.main()
