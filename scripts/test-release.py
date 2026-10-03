import importlib.util
import pathlib
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("release", pathlib.Path(__file__).with_name("release.py"))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ReleaseTests(unittest.TestCase):
    def test_version_and_tag_must_agree(self):
        current = release.version()
        self.assertEqual(release.version(tag="v" + current), current)
        for invalid in (current, "v999.0.0", "v" + current + "-beta.1", "main"):
            with self.assertRaises(ValueError):
                release.version(tag=invalid)

    def test_missing_or_empty_asset_prevents_publication(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            for name in release.asset_names("1.2.3"):
                (root / name).write_bytes(b"installer")
            release.checksum_assets(root, "1.2.3")
            sums = (root / "SHA256SUMS").read_bytes()
            self.assertEqual(len(sums.splitlines()), 5)
            release.checksum_assets(root, "1.2.3")
            self.assertEqual((root / "SHA256SUMS").read_bytes(), sums)
            missing = root / "Aieyes-1.2.3-macos-arm64.dmg"
            missing.unlink()
            with self.assertRaises(ValueError):
                release.checksum_assets(root, "1.2.3")
            missing.touch()
            with self.assertRaises(ValueError):
                release.checksum_assets(root, "1.2.3")

    def test_collector_rejects_ambiguous_stale_packages(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            (root / "dist").mkdir()
            (root / "dist/one.dmg").write_bytes(b"package")
            release.collect("macos", "arm64", "1.2.3", root / "assets", root)
            self.assertTrue((root / "assets/Aieyes-1.2.3-macos-arm64.dmg").is_file())
            (root / "dist/two.dmg").write_bytes(b"stale")
            with self.assertRaises(ValueError):
                release.collect("macos", "arm64", "1.2.3", root / "assets", root)


if __name__ == "__main__":
    unittest.main()
