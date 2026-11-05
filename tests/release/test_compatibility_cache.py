"""Historical cache repair cannot upload objects or switch release channels."""
import contextlib
import importlib.util
import io
import os
import pathlib
import sys
import unittest
from unittest.mock import Mock, patch

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts/release'))
SPEC = importlib.util.spec_from_file_location(
    'compatibility_cache', ROOT / 'scripts/release/refresh_compatibility_cache.py')
CACHE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CACHE)


class CompatibilityCacheTest(unittest.TestCase):
    def test_refresh_only_invalidates_the_fixed_upgrade_fixtures(self):
        store = Mock(spec=['refresh_many'])
        CACHE.refresh(store)
        store.refresh_many.assert_called_once()
        keys = store.refresh_many.call_args.args[0]
        self.assertEqual(len(keys), 10)
        self.assertEqual(len(set(keys)), len(keys))
        self.assertIn('skills/0.5.5/release.json', keys)
        self.assertIn('releases/0.5.5/dt-cli-macos-x64.zip', keys)
        for key in keys:
            self.assertTrue(key.startswith((
                'releases/0.4.1/', 'skills/0.4.4/',
                'releases/0.5.5/', 'skills/0.5.5/')))
            self.assertNotIn('stable', key)
            self.assertNotIn('..', key)

    def test_invalid_configuration_cannot_create_a_store_or_refresh(self):
        with patch.dict(os.environ, {
            'QINIU_PUBLIC_BASE_URL': 'https://invalid.example/',
            'QINIU_BUCKET': 'fixture', 'QINIU_REGION': 'z0',
        }), patch.object(CACHE, 'QiniuStore') as store, contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit):
                CACHE.main()
        store.assert_not_called()

    def test_sdk_failure_is_fatal_without_exposing_signed_urls(self):
        stderr = io.StringIO()
        with patch.dict(os.environ, {
            'QINIU_PUBLIC_BASE_URL': 'https://cdn.jmj1995.com/dt-cli/',
            'QINIU_BUCKET': 'fixture', 'QINIU_REGION': 'z0',
        }), patch.object(CACHE, 'QiniuStore') as store, contextlib.redirect_stderr(stderr):
            store.return_value.refresh_many.side_effect = RuntimeError('signed-url-secret')
            with self.assertRaises(SystemExit) as raised:
                CACHE.main()
        self.assertEqual(raised.exception.code, 1)
        self.assertIn('COMPATIBILITY_CACHE_REFRESH_FAILED', stderr.getvalue())
        self.assertNotIn('signed-url-secret', stderr.getvalue())


if __name__ == '__main__':
    unittest.main()
