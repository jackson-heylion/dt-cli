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
        data = self.objects.get(key)
        if data is not None and len(data) > maximum:
            raise ValueError('oversized readback')
        return data

    def refresh_many(self, keys):
        pass

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
            self.assertEqual(store.uploads[-2], (RELEASE.STABLE_ARCHIVE_KEY, False))
            release = json.loads(store.objects[channel['releaseKey']])
            self.assertEqual(store.objects[RELEASE.STABLE_ARCHIVE_KEY], store.objects[release['key']])
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
            self.assertNotIn(RELEASE.STABLE_ARCHIVE_KEY, store.objects)
            self.assertEqual(store.objects['channels/stable.json'], b'unchanged native channel')

    def test_failed_stable_download_preserves_channel_and_publication_resumes(self):
        with tempfile.TemporaryDirectory() as folder:
            tree = pathlib.Path(folder) / 'tree'
            channel = self.prepare(tree)
            for fail_key in (RELEASE.STABLE_ARCHIVE_KEY, 'channels/skill-stable.json'):
                with self.subTest(fail_key=fail_key):
                    store = MemoryStore(fail_key=fail_key)
                    store.objects[RELEASE.STABLE_ARCHIVE_KEY] = b'old verified package'
                    with self.assertRaises(ValueError):
                        RELEASE.publish(tree, store)
                    self.assertNotIn('channels/skill-stable.json', store.objects)
                    if fail_key == RELEASE.STABLE_ARCHIVE_KEY:
                        self.assertEqual(store.objects[RELEASE.STABLE_ARCHIVE_KEY], b'old verified package')
                    immutable_uploads = [key for key, immutable in store.uploads if immutable]
                    store.fail_key = None
                    result = RELEASE.publish(tree, store)
                    self.assertEqual([key for key, immutable in store.uploads if immutable], immutable_uploads)
                    self.assertEqual(json.loads(store.objects['channels/skill-stable.json']), channel)
                    self.assertEqual(store.objects[result['stableKey']], store.objects[result['key']])

    def test_stale_stable_download_readback_cannot_switch_the_channel(self):
        with tempfile.TemporaryDirectory() as folder:
            tree = pathlib.Path(folder) / 'tree'
            self.prepare(tree)

            class StaleStore(MemoryStore):
                def read(self, key, maximum):
                    if key == RELEASE.STABLE_ARCHIVE_KEY:
                        return b'cached old package'
                    return super().read(key, maximum)

            store = StaleStore()
            with self.assertRaisesRegex(ValueError, 'Public Skill readback differs'):
                RELEASE.publish(tree, store)
            self.assertNotIn('channels/skill-stable.json', store.objects)

    def test_existing_channel_repairs_missing_stable_download(self):
        with tempfile.TemporaryDirectory() as folder:
            tree = pathlib.Path(folder) / 'tree'
            self.prepare(tree)
            store = MemoryStore()
            RELEASE.publish(tree, store)
            del store.objects[RELEASE.STABLE_ARCHIVE_KEY]
            uploads = store.uploads.copy()
            RELEASE.publish(tree, store)
            self.assertEqual(store.uploads, uploads + [(RELEASE.STABLE_ARCHIVE_KEY, False)])

    def test_larger_previous_stable_zip_is_replaced_after_new_version_verifies(self):
        with tempfile.TemporaryDirectory() as folder:
            tree = pathlib.Path(folder) / 'tree'
            channel = self.prepare(tree)
            release = json.loads((tree / channel['releaseKey']).read_bytes())
            old_zip = b'x' * (release['bytes'] + 1)

            class PropagatingStore(MemoryStore):
                stable_reads = 0

                def read(self, key, maximum):
                    if key == RELEASE.STABLE_ARCHIVE_KEY:
                        self.stable_reads += 1
                        if self.stable_reads <= 2:
                            if len(old_zip) > maximum:
                                raise ValueError('oversized readback')
                            return old_zip
                    return super().read(key, maximum)

            store = PropagatingStore()
            store.objects[RELEASE.STABLE_ARCHIVE_KEY] = old_zip
            with patch.object(RELEASE.time, 'sleep'):
                RELEASE.publish(tree, store, wait_for_readback=True)
            self.assertEqual(store.objects[RELEASE.STABLE_ARCHIVE_KEY], store.objects[release['key']])

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
                    store.objects[RELEASE.STABLE_ARCHIVE_KEY] = b'previous stable package'
                    with self.assertRaises(ValueError):
                        RELEASE.publish(tree, store)
                    self.assertEqual(store.uploads, [])
                    self.assertEqual(store.objects['channels/skill-stable.json'], before)
                    self.assertEqual(store.objects[RELEASE.STABLE_ARCHIVE_KEY], b'previous stable package')

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
            self.assertNotIn(RELEASE.STABLE_ARCHIVE_KEY, store.objects)
            self.assertNotIn(('channels/skill-stable.json', False), store.uploads)


class PublicCacheRefreshTest(unittest.TestCase):
    def store(self, upload_status=200, refresh_code=200):
        store = RELEASE.QiniuStore.__new__(RELEASE.QiniuStore)
        store.base = 'https://cdn.fixture.invalid/dt-cli/'
        store.prefix = 'dt-cli/'
        store.bucket = 'fixture'
        store.regions = []
        store.auth = SimpleNamespace(upload_token=Mock(return_value='synthetic-upload-token'),
                                     token_of_request=Mock(return_value='synthetic-management-token'))
        store.management_auth = Mock()
        cache = {'cacheControls': [{'time': 1, 'timeunit': 5, 'type': 'all', 'rule': '*'}],
                 'ignoreParam': False, 'ignoreParams': ['fixture'], 'includeParams': []}
        store.requests = SimpleNamespace(
            get=Mock(return_value=SimpleNamespace(status_code=200, json=lambda: {'cache': cache})),
            put=Mock(return_value=SimpleNamespace(status_code=200, json=lambda: {'code': 200})),
            post=Mock(return_value=SimpleNamespace(status_code=200)),
            head=Mock(return_value=SimpleNamespace(status_code=200, headers={'Cache-Control': 'no-cache, max-age=0'})))
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
        self.assertIn('/cacheControl/', store.requests.post.call_args.args[0])

    def test_stale_browser_cache_header_is_rejected_after_refresh(self):
        store, _ = self.store()
        store.requests.head.return_value.headers = {'Cache-Control': 'max-age=2592000'}
        with patch.object(RELEASE.time, 'sleep'), self.assertRaisesRegex(ValueError, 'stale client caching'):
            store.upload('channels/skill-stable.json', pathlib.Path('fixture.json'), immutable=False)

    def test_cache_exception_preserves_unrelated_rules_and_query_configuration(self):
        store, _ = self.store()
        previous = store.requests.get.return_value.json()['cache']
        store.configure_mutable_cache()
        body = store.requests.put.call_args.kwargs['json']
        self.assertEqual(body['cacheControls'][1:], previous['cacheControls'])
        self.assertEqual(body['cacheControls'][0], {'time': 0, 'timeunit': 0, 'type': 'path',
                         'rule': '/dt-cli/channels;/dt-cli/skills/stable'})
        for field in ('ignoreParam', 'ignoreParams', 'includeParams'):
            self.assertEqual(body[field], previous[field])
        store.configure_mutable_cache()
        store.requests.put.assert_called_once()

    def test_existing_cache_exception_is_not_rewritten(self):
        store, _ = self.store()
        store.requests.get.return_value.json()['cache']['cacheControls'].insert(0,
            {'time': 0, 'timeunit': 0, 'type': 'path', 'rule': '/dt-cli/channels;/dt-cli/skills/stable'})
        store.configure_mutable_cache()
        store.requests.put.assert_not_called()

    def test_failed_cache_policy_does_not_report_success(self):
        store, manager = self.store()
        store.requests.post.return_value.status_code = 403
        with self.assertRaisesRegex(ValueError, 'cache policy update failed'):
            store.upload('channels/skill-stable.json', pathlib.Path('fixture.json'), immutable=False)
        manager.refresh_urls.assert_not_called()

    def test_existing_immutable_upload_also_purges_cached_not_found(self):
        store, manager = self.store(upload_status=614)
        store.upload('skills/0.4.4/release.json', pathlib.Path('fixture.json'), immutable=True)
        manager.refresh_urls.assert_not_called()
        store.refresh_many(['skills/0.4.4/release.json', 'skills/0.4.4/dt-cli-skill.zip'])
        manager.refresh_urls.assert_called_once_with([
            'https://cdn.fixture.invalid/dt-cli/skills/0.4.4/release.json',
            'https://cdn.fixture.invalid/dt-cli/skills/0.4.4/dt-cli-skill.zip'])

    def test_rejected_refresh_is_not_reported_as_completed_publication(self):
        store, _ = self.store(refresh_code=403)
        with self.assertRaisesRegex(ValueError, 'Qiniu CDN refresh failed'):
            store.upload('channels/skill-stable.json', pathlib.Path('fixture.json'), immutable=False)


if __name__ == '__main__':
    unittest.main()
