"""Release identity checks: python3 tests/versioning.py."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("version", Path(__file__).resolve().parents[1] / "scripts/version.py")
version = importlib.util.module_from_spec(spec)
spec.loader.exec_module(version)


class Versioning(unittest.TestCase):
    def test_release_tag_matches_application_version(self):
        for value in ("0.1.0", "1.0.0", "0.2.0-rc.1"):
            self.assertEqual(version.build_metadata(value, "a" * 40, f"refs/tags/v{value}")["build_version"], value)

    def test_wrong_or_invalid_release_tag_is_rejected(self):
        for ref in ("refs/tags/v0.2.0", "refs/tags/0.1.0", "refs/tags/v0.1.0-extra"):
            with self.assertRaises(ValueError):
                version.build_metadata("0.1.0", "a" * 40, ref)

    def test_development_builds_have_commit_identity(self):
        self.assertEqual(version.build_metadata("0.1.0", "abcdef0123456789", "refs/heads/main")["build_version"], "0.1.0+dev.abcdef012345")

    def test_invalid_semver_is_rejected(self):
        for value in ("01.1.0", "0.1", "0.1.0-01", "0.1.0+unexpected"):
            with self.assertRaises(ValueError):
                version.build_metadata(value, "a" * 40, "")


if __name__ == "__main__":
    unittest.main()
