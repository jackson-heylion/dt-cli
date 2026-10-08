"""Universal Skill publishes independently while native stable stays unchanged."""
import importlib.util
import json
import pathlib
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts/release'))
SPEC = importlib.util.spec_from_file_location('publish_skill', ROOT / 'scripts/release/publish_skill.py')
RELEASE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RELEASE)


class MemoryStore:
    def __init__(self, fail_key=None):
        self.objects = {'channels/stable.json': b'unchanged native channel'}
        self.uploads = []
        self.fail_key = fail_key

    def read(self, key, maximum):
        return self.objects.get(key)

    def upload(self, key, path, immutable):
        if key == self.fail_key:
            raise ValueError('interrupted')
        self.uploads.append((key, immutable))
        self.objects[key] = path.read_bytes()


class SkillReleaseTest(unittest.TestCase):
    def prepare(self, output):
        def source_git(argv, **kwargs):
            return b'' if 'status' in argv else 'a' * 40 + '\n'
        with patch.object(RELEASE.subprocess, 'check_output', source_git):
            return RELEASE.assemble(output)

    def test_universal_release_switches_only_skill_channel_last(self):
        with tempfile.TemporaryDirectory() as folder:
            tree = pathlib.Path(folder) / 'tree'
            channel = self.prepare(tree)
            store = MemoryStore()
            result = RELEASE.publish(tree, store)
            self.assertTrue(result['published'])
            self.assertFalse(result['nativeCliChanged'])
            self.assertEqual(store.objects['channels/stable.json'], b'unchanged native channel')
            self.assertEqual(store.uploads[-1], ('channels/skill-stable.json', False))
            self.assertEqual(json.loads(store.objects['channels/skill-stable.json']), channel)
            uploads = store.uploads.copy()
            RELEASE.publish(tree, store)
            self.assertEqual(store.uploads, uploads)

    def test_failed_immutable_upload_cannot_switch_skill_or_native_channel(self):
        with tempfile.TemporaryDirectory() as folder:
            tree = pathlib.Path(folder) / 'tree'
            channel = self.prepare(tree)
            store = MemoryStore(fail_key=channel['releaseKey'])
            with self.assertRaises(ValueError):
                RELEASE.publish(tree, store)
            self.assertNotIn('channels/skill-stable.json', store.objects)
            self.assertEqual(store.objects['channels/stable.json'], b'unchanged native channel')

    def test_conflicting_skill_version_preserves_current_channels(self):
        with tempfile.TemporaryDirectory() as folder:
            tree = pathlib.Path(folder) / 'tree'
            channel = self.prepare(tree)
            release = json.loads((tree / channel['releaseKey']).read_bytes())
            store = MemoryStore()
            store.objects[release['key']] = b'prior immutable package'
            with self.assertRaises(ValueError):
                RELEASE.publish(tree, store)
            self.assertEqual(store.objects[release['key']], b'prior immutable package')
            self.assertEqual(store.uploads, [])

    def test_newer_or_conflicting_skill_channel_rejects_before_upload(self):
        with tempfile.TemporaryDirectory() as folder:
            tree = pathlib.Path(folder) / 'tree'
            channel = self.prepare(tree)
            for previous in (dict(channel, version='0.5.0'),
                             dict(channel, releaseSha256='b' * 64)):
                with self.subTest(previous=previous):
                    store = MemoryStore()
                    before = json.dumps(previous).encode()
                    store.objects['channels/skill-stable.json'] = before
                    with self.assertRaises(ValueError):
                        RELEASE.publish(tree, store)
                    self.assertEqual(store.uploads, [])
                    self.assertEqual(store.objects['channels/skill-stable.json'], before)

    def test_competing_publisher_keeps_its_channel(self):
        with tempfile.TemporaryDirectory() as folder:
            tree = pathlib.Path(folder) / 'tree'
            channel = self.prepare(tree)
            competing = json.dumps(dict(channel, version='0.5.0')).encode()

            class RacingStore(MemoryStore):
                def upload(self, key, path, immutable):
                    super().upload(key, path, immutable)
                    if key == channel['releaseKey']:
                        self.objects['channels/skill-stable.json'] = competing

            store = RacingStore()
            with self.assertRaises(ValueError):
                RELEASE.publish(tree, store)
            self.assertEqual(store.objects['channels/skill-stable.json'], competing)
            self.assertNotIn(('channels/skill-stable.json', False), store.uploads)


if __name__ == '__main__':
    unittest.main()
