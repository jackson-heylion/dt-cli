"""Release aggregation and publication failures preserve the existing stable channel."""
import copy
import hashlib
import importlib.util
import json
import pathlib
import tempfile
import unittest
import zipfile
from unittest.mock import patch

ROOT = pathlib.Path(__file__).resolve().parents[2]


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / f'scripts/release/{name}.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


INDEX = load('index')
PUBLISH = load('publish_qiniu')


class MemoryStore:
    def __init__(self, objects=None, fail_key=None):
        self.objects = dict(objects or {})
        self.uploads = []
        self.fail_key = fail_key

    def read(self, key, maximum):
        result = self.objects.get(key)
        if result is not None and len(result) > maximum:
            raise ValueError('oversized readback')
        return result

    def upload(self, key, path, immutable):
        if key == self.fail_key:
            raise ValueError('simulated interruption')
        if immutable and key in self.objects:
            raise ValueError('immutable overwrite')
        self.uploads.append((key, immutable))
        self.objects[key] = path.read_bytes()


class DistributionTest(unittest.TestCase):
    def fixture(self, root):
        artifacts = root / 'artifacts'
        configuration = (ROOT / 'skills/dt-cli/scripts/distribution.json').read_bytes()
        for target in json.loads((ROOT / 'catalog/release-targets.json').read_bytes())['targets']:
            name = target['name']
            folder = artifacts / name
            folder.mkdir(parents=True)
            binary = ('synthetic ' + name).encode()
            manifest = dict(manifestSchemaVersion=1, releaseType='release', version='0.4.1',
                            **{field: target[field] for field in ('os', 'architecture', 'binary')},
                            sha256=INDEX.digest(binary), buildCommit='a' * 40, buildTarget=target['target'],
                            localDevelopment=False, catalogVersion='1.3.1', catalogDigest='b' * 64,
                            sourceVersion='fixture', profileFormat=1, credentialFormat=1,
                            minimumInstallerSchema=1, minimumLauncherSchema=1, nativeProbe='performed')
            archive = folder / f'dt-cli-{name}.zip'
            with zipfile.ZipFile(archive, 'w') as package:
                package.writestr(target['binary'], binary)
                package.writestr('manifest.json', json.dumps(manifest))
                package.writestr('install.sh', 'synthetic installer')
                package.writestr('install.ps1', 'synthetic installer')
            archive.with_suffix('.manifest.json').write_text(json.dumps(manifest))
            digest = INDEX.digest(archive.read_bytes())
            archive.with_suffix('.zip.sha256').write_text(digest + '  ' + archive.name)
            evidence = dict(version='0.4.1', buildCommit='a' * 40, catalogDigest='b' * 64,
                            sourceVersion='fixture', target=target['target'], archiveSha256=digest, nativeProbe='performed')
            (folder / f'dt-cli-{name}.native-check.json').write_text(json.dumps(evidence))
            with zipfile.ZipFile(folder / 'dt-cli-skill.zip', 'w') as package:
                package.writestr(zipfile.ZipInfo('dt-cli/scripts/distribution.json', (2026, 1, 1, 0, 0, 0)), configuration)
        return artifacts

    def test_missing_target_or_wrong_native_provenance_creates_no_channel(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            artifacts = self.fixture(root)
            archive = artifacts / 'windows-x64/dt-cli-windows-x64.zip'
            original = archive.read_bytes()
            archive.unlink()
            with self.assertRaises(ValueError):
                INDEX.assemble(artifacts, root / 'missing', 1)
            self.assertFalse((root / 'missing').exists())
            archive.write_bytes(original)
            evidence_path = artifacts / 'windows-x64/dt-cli-windows-x64.native-check.json'
            evidence = json.loads(evidence_path.read_bytes())
            evidence['buildCommit'] = 'c' * 40
            evidence_path.write_text(json.dumps(evidence))
            with self.assertRaises(ValueError):
                INDEX.assemble(artifacts, root / 'mismatch', 1)
            self.assertFalse((root / 'mismatch').exists())

    def test_incompatible_packaged_skill_cannot_create_a_stable_channel(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            artifacts = self.fixture(root)
            incompatible = dict(INDEX.COMPATIBILITY, maximumSkillVersionExclusive='0.5.0')
            with patch.object(INDEX, 'COMPATIBILITY', incompatible):
                with self.assertRaisesRegex(ValueError, 'Skill is outside'):
                    INDEX.assemble(artifacts, root / 'incompatible', 1)
            self.assertFalse((root / 'incompatible').exists())

    def test_complete_distribution_switches_stable_last_and_is_repeatable(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            tree = root / 'tree'
            stable = INDEX.assemble(self.fixture(root), tree, 2)
            store = MemoryStore()
            result = PUBLISH.publish(tree, store)
            self.assertTrue(result['published'])
            self.assertEqual(store.uploads[-2:], [('channels/stable.json', False), ('channels/native-stable.json', False)])
            self.assertTrue(all(immutable for _, immutable in store.uploads[:-2]))
            self.assertEqual(json.loads(store.objects['channels/native-stable.json']), stable)
            legacy = json.loads(store.objects['channels/stable.json'])
            release = json.loads(store.objects[stable['releaseKey']])
            legacy_release = json.loads(store.objects[legacy['releaseKey']])
            self.assertEqual(len(release['packages']), 3)
            self.assertEqual([p['target'] for p in legacy_release['packages']], ['aarch64-apple-darwin', 'x86_64-pc-windows-msvc'])
            self.assertEqual(legacy_release, dict(release, packages=[p for p in release['packages'] if p['target'] != 'x86_64-apple-darwin']))
            uploads = copy.copy(store.uploads)
            self.assertTrue(PUBLISH.publish(tree, store)['stableVerified'])
            self.assertEqual(store.uploads, uploads)

    def test_interrupted_upload_or_conflicting_immutable_key_preserves_old_stable(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            tree = root / 'tree'
            INDEX.assemble(self.fixture(root), tree, 2)
            old = json.dumps(dict(sequence=1, version='0.4.0')).encode()
            for store in (MemoryStore({'channels/stable.json': old}, fail_key='releases/0.4.1/dt-cli-windows-x64.zip'),
                          MemoryStore({'channels/stable.json': old, 'releases/0.4.1/dt-cli-macos-arm64.zip': b'wrong'})):
                with self.assertRaises(ValueError):
                    PUBLISH.publish(tree, store)
                self.assertEqual(store.objects['channels/stable.json'], old)
                self.assertNotIn(('channels/stable.json', False), store.uploads)

    def test_sequence_reuse_and_downgrade_are_rejected_before_upload(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            tree = root / 'tree'
            INDEX.assemble(self.fixture(root), tree, 2)
            for previous in (dict(sequence=3, version='0.4.1'), dict(sequence=2, version='0.4.0'), dict(sequence=1, version='0.5.0')):
                store = MemoryStore({'channels/stable.json': json.dumps(previous).encode()})
                with self.assertRaises(ValueError):
                    PUBLISH.publish(tree, store)
                self.assertEqual(store.uploads, [])

    def test_bad_local_package_is_rejected_before_any_upload(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            tree = root / 'tree'
            INDEX.assemble(self.fixture(root), tree, 2)
            (tree / 'releases/0.4.1/dt-cli-macos-arm64.zip').write_bytes(b'corrupt')
            store = MemoryStore()
            with self.assertRaises(ValueError):
                PUBLISH.publish(tree, store)
            self.assertEqual(store.uploads, [])

    def test_intel_artifact_is_required_and_all_three_skills_must_match(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            artifacts = self.fixture(root)
            intel = artifacts / 'macos-x64/dt-cli-macos-x64.zip'
            original = intel.read_bytes()
            intel.unlink()
            with self.assertRaises(ValueError):
                INDEX.assemble(artifacts, root / 'missing-intel', 2)
            self.assertFalse((root / 'missing-intel').exists())
            intel.write_bytes(original)
            with zipfile.ZipFile(artifacts / 'windows-x64/dt-cli-skill.zip', 'a') as package:
                package.writestr('dt-cli/different.txt', 'third target differs')
            with self.assertRaises(ValueError):
                INDEX.assemble(artifacts, root / 'different-skill', 2)
            self.assertFalse((root / 'different-skill').exists())

    def test_interrupted_channel_switch_resumes_without_replacing_immutable_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            tree = root / 'tree'
            stable = INDEX.assemble(self.fixture(root), tree, 2)
            store = MemoryStore(fail_key='channels/native-stable.json')
            with self.assertRaises(ValueError):
                PUBLISH.publish(tree, store)
            self.assertIn('channels/stable.json', store.objects)
            self.assertNotIn('channels/native-stable.json', store.objects)
            uploads = store.uploads.copy()
            store.fail_key = None
            self.assertTrue(PUBLISH.publish(tree, store)['legacyStableVerified'])
            self.assertEqual(store.uploads, uploads + [('channels/native-stable.json', False)])
            self.assertEqual(json.loads(store.objects['channels/native-stable.json']), stable)

    def test_legacy_projection_cannot_publish_different_packages(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            tree = root / 'tree'
            INDEX.assemble(self.fixture(root), tree, 2)
            channel_path = tree / 'channels/stable.json'
            channel = json.loads(channel_path.read_bytes())
            index = tree / channel['releaseKey']
            release = json.loads(index.read_bytes())
            release['packages'].pop()
            index.write_text(json.dumps(release))
            channel['releaseSha256'] = PUBLISH.sha(index.read_bytes())
            channel_path.write_text(json.dumps(channel))
            store = MemoryStore()
            with self.assertRaises(ValueError):
                PUBLISH.publish(tree, store)
            self.assertEqual(store.uploads, [])


if __name__ == '__main__':
    unittest.main()
