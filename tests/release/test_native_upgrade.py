import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'scripts/release'))
from native_upgrade import Stable, TARGET, previous_windows, require_verified_baseline

CHANNEL = 'channels/native-stable.json'


def encoded(value):
    return json.dumps(value).encode()


def sha256(raw):
    return hashlib.sha256(raw).hexdigest()


def pointer(tag, sequence, digest):
    return dict(schemaVersion=1, version=tag, sequence=sequence,
                releaseKey=f'releases/{tag}/native-release.json', releaseSha256=digest)


class NativeUpgradeTests(unittest.TestCase):
    def test_previous_windows_requires_the_public_index_and_exact_archive(self):
        archive = b'published archive'
        tag = '0.8.1'
        release_key = f'releases/{tag}/native-release.json'
        zip_key = f'releases/{tag}/dt-cli-windows-x64.zip'
        release = encoded(dict(schemaVersion=1, version=tag, buildCommit='a' * 40, catalogDigest='b' * 64,
                               packages=[dict(target=TARGET, os='Windows', architecture='x86_64',
                                              key=zip_key, sha256=sha256(archive), bytes=len(archive))]))
        public = {CHANNEL: encoded(pointer(tag, 54, sha256(release))), release_key: release, zip_key: archive}

        def fetch(key, limit):
            self.assertLessEqual(len(public[key]), limit)
            return public[key]

        self.assertEqual(previous_windows(fetch, CHANNEL).archive, archive)
        public[zip_key] = b'x' * len(archive)
        with self.assertRaisesRegex(ValueError, 'package digest or length'):
            previous_windows(fetch, CHANNEL)
        public[release_key] = encoded({})
        with self.assertRaisesRegex(ValueError, 'index digest'):
            previous_windows(fetch, CHANNEL)

    def test_promotion_requires_evidence_for_the_current_stable_or_exact_rerun(self):
        prior = Stable(54, '0.8.1', 'a' * 64)
        candidate = Stable(56, '0.8.3', 'b' * 64)
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            channel = root / CHANNEL
            channel.parent.mkdir()
            channel.write_bytes(encoded(pointer(candidate.version, candidate.sequence, candidate.release_sha256)))
            evidence = root / 'releases/0.8.3/dt-cli-windows-x64.native-check.json'
            evidence.parent.mkdir(parents=True)
            evidence.write_bytes(encoded(dict(windowsUpgrade=dict(
                candidateVersion=candidate.version, verified=True, previousStable=prior.evidence()))))
            require_verified_baseline(root, prior)
            require_verified_baseline(root, candidate)
            with self.assertRaisesRegex(ValueError, 'Stable changed'):
                require_verified_baseline(root, Stable(55, '0.8.2', 'c' * 64))
            evidence.write_bytes(b'{}')
            with self.assertRaisesRegex(ValueError, 'Missing Windows'):
                require_verified_baseline(root, prior)
