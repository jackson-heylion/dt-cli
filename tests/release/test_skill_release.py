"""Universal Skill publishes independently while native stable stays unchanged."""
import importlib.util
import json
import pathlib
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts/release'))
SPEC = importlib.util.spec_from_file_location('publish_skill', ROOT / 'scripts/release/publish_skill.py')
RELEASE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RELEASE)


class MemoryStore:
    def __init__(self, fail_key=None):
        self.objects = {'channels/stable.json': b'unchanged native channel',
                        'channels/native-stable.json': b'unchanged three-platform channel'}
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
            self.assertEqual(store.objects['channels/native-stable.json'], b'unchanged three-platform channel')
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
            for previous in (dict(channel, version='0.6.0', releaseKey='skills/0.6.0/release.json'),
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


class PublicCacheRefreshTest(unittest.TestCase):
    def store(self, upload_status=200, refresh_code=200):
        store = RELEASE.QiniuStore.__new__(RELEASE.QiniuStore)
        store.base = 'https://cdn.fixture.invalid/dt-cli/'
        store.prefix = 'dt-cli/'
        store.bucket = 'fixture'
        store.regions = []
        store.auth = SimpleNamespace(upload_token=Mock(return_value='synthetic-upload-token'))
        manager = SimpleNamespace(refresh_urls=Mock(return_value=(
            {'code': refresh_code}, SimpleNamespace(status_code=200))))
        store.qiniu = SimpleNamespace(
            put_file_v2=Mock(return_value=(
                {'key': 'dt-cli/channels/skill-stable.json'} if upload_status == 200 else None,
                SimpleNamespace(status_code=upload_status))),
            CdnManager=Mock(return_value=manager))
        return store, manager

    def test_channel_upload_purges_only_its_exact_public_url(self):
        store, manager = self.store()
        store.upload('channels/skill-stable.json', pathlib.Path('fixture.json'), immutable=False)
        self.assertEqual(manager.server, 'https://fusion.qiniuapi.com')
        manager.refresh_urls.assert_called_once_with(['https://cdn.fixture.invalid/dt-cli/channels/skill-stable.json'])

    def test_existing_immutable_upload_also_purges_cached_not_found(self):
        store, manager = self.store(upload_status=614)
        store.upload('skills/0.4.4/release.json', pathlib.Path('fixture.json'), immutable=True)
        manager.refresh_urls.assert_called_once_with(['https://cdn.fixture.invalid/dt-cli/skills/0.4.4/release.json'])

    def test_rejected_refresh_is_not_reported_as_completed_publication(self):
        store, _ = self.store(refresh_code=403)
        with self.assertRaisesRegex(ValueError, 'Qiniu CDN refresh failed'):
            store.upload('channels/skill-stable.json', pathlib.Path('fixture.json'), immutable=False)


if __name__ == '__main__':
    unittest.main()
