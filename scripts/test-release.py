import importlib.util
import pathlib
import tempfile
import unittest
import base64
import json
import plistlib
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("release", pathlib.Path(__file__).with_name("release.py"))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ReleaseTests(unittest.TestCase):
    def test_manifest_uses_utf8_on_non_utf8_hosts(self):
        original_open = pathlib.Path.open
        def windows_open(path, mode='r', buffering=-1, encoding=None, errors=None, newline=None):
            if 'b' not in mode and encoding in (None, 'locale'):
                encoding = 'cp1252'
            return original_open(path, mode, buffering, encoding, errors, newline)
        with tempfile.TemporaryDirectory() as temporary, patch.object(pathlib.Path, 'open', windows_open):
            root = pathlib.Path(temporary)
            self.assets(root)
            release.checksum_assets(root, '1.2.3')
            self.assertEqual(json.loads((root/'latest.json').read_bytes())['version'], '1.2.3')

    def assets(self, root, version='1.2.3'):
        for name in release.installer_names(version):
            (root / name).write_bytes(b'installer')
            if '-macos-' not in name:
                (root / (name + '.sig')).write_text(base64.b64encode(f'untrusted comment: fixture\nfixture\ntrusted comment: version:{version}\nfixture'.encode()).decode())
        build = plistlib.loads((release.ROOT/'apps/macos/Info.plist').read_bytes())['CFBundleVersion']
        signature = base64.b64encode(bytes(64)).decode()
        for arch in ('arm64', 'x64'):
            name = f'Aieyes-{version}-macos-{arch}.dmg'
            (root / f'appcast-macos-{arch}.xml').write_text(f'<rss xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle"><channel><item><sparkle:version>{build}</sparkle:version><sparkle:shortVersionString>{version}</sparkle:shortVersionString><enclosure url="https://github.com/JesmonX/Aieyes/releases/download/v{version}/{name}" length="9" sparkle:edSignature="{signature}" /></item></channel></rss><!-- sparkle-signatures: fixture -->')
        release.update_manifest(root, version)
    def test_version_and_tag_must_agree(self):
        current = release.version()
        self.assertEqual(release.version(tag="v" + current), current)
        for invalid in (current, "v999.0.0", "v" + current + "-beta.1", "main"):
            with self.assertRaises(ValueError):
                release.version(tag=invalid)

    def test_missing_or_empty_asset_prevents_publication(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            self.assets(root)
            release.checksum_assets(root, "1.2.3")
            sums = (root / "SHA256SUMS").read_bytes()
            self.assertEqual(len(sums.splitlines()), 11)
            release.checksum_assets(root, "1.2.3")
            self.assertEqual((root / "SHA256SUMS").read_bytes(), sums)
            missing = root / "Aieyes-1.2.3-macos-arm64.dmg"
            missing.unlink()
            with self.assertRaises(ValueError):
                release.checksum_assets(root, "1.2.3")
            missing.touch()
            with self.assertRaises(ValueError):
                release.checksum_assets(root, "1.2.3")

    def test_feed_cannot_point_at_another_release_or_omit_an_installer(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            self.assets(root)
            manifest_path = root/'latest.json'
            original = manifest_path.read_text(encoding="utf-8")
            manifest = json.loads(original)
            manifest['platforms']['linux-x86_64-deb']['url'] = 'https://example.com/other.deb'
            manifest_path.write_text(json.dumps(manifest))
            with self.assertRaises(ValueError):
                release.checksum_assets(root, '1.2.3')
            manifest_path.write_text(original, encoding="utf-8")
            (root/'appcast-macos-x64.xml').write_text((root/'appcast-macos-x64.xml').read_text(encoding="utf-8").replace('<sparkle:version>', '<sparkle:version>9'))
            with self.assertRaises(ValueError):
                release.checksum_assets(root, '1.2.3')

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
