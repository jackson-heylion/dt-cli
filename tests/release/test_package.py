"""Internal packaging checks; native execution fixtures do not prove platform acceptance."""
import contextlib
import io
import json
import pathlib
import runpy
import struct
import subprocess
import tempfile
import unittest
import zipfile
from unittest.mock import patch
SCRIPT = pathlib.Path(__file__).resolve().parents[2] / 'scripts/release/package.py'
PACKAGE = runpy.run_path(str(SCRIPT))

class PackageTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root = pathlib.Path(self.temp.name); self.binary = self.root/'dt-cli'
        header = bytearray(64);header[:4]=b'\xcf\xfa\xed\xfe';struct.pack_into('<I', header, 4, 0x0100000c)
        self.binary.write_bytes(header+b'fixture')
        self.details = {'cliVersion':'0.3.0','buildCommit':'b'*40,'buildTarget':'aarch64-apple-darwin','localDevelopment':False,'catalogVersion':'1.2.0','catalogDigest':'a'*64,'sourceVersion':'fixture'}
        self.metadata=self.root/'dt-cli.build.json';self.metadata.write_text(json.dumps(self.details))
        self.output=self.root/'release.zip'
    def command(self,args,**kwargs):
        if args[-1]=='version':return subprocess.CompletedProcess(args,0,json.dumps({'ok':True,'data':self.details}),'')
        raise AssertionError('Internal packaging must not invoke system signature tools')
    def package(self,kind):
        with contextlib.redirect_stdout(io.StringIO()):
            return PACKAGE['package'](self.binary,self.output,kind)
    def test_cross_compiled_candidate_never_executes_binary_or_signature_tools(self):
        self.details['buildCommit']+='\u002bdirty';self.metadata.write_text(json.dumps(self.details))
        with patch('subprocess.run',side_effect=AssertionError('candidate must not execute')):
            manifest=self.package('candidate')
        self.assertEqual(manifest['nativeProbe'],'not-performed');self.assertEqual(manifest['releaseType'],'candidate')
        with zipfile.ZipFile(self.output) as archive:self.assertEqual(set(archive.namelist()),{'dt-cli','manifest.json','install.sh','install.ps1'})
        self.assertTrue(self.output.with_suffix('.zip.sha256').is_file())
    def test_internal_release_needs_only_metadata_and_native_probe(self):
        with patch('platform.system',return_value='Darwin'),patch('subprocess.run',side_effect=self.command):manifest=self.package('release')
        self.assertEqual(manifest['buildCommit'],'b'*40);self.assertEqual(manifest['nativeProbe'],'performed')
    def test_unsigned_windows_release_uses_native_probe_without_signtool(self):
        header=bytearray(128);header[:2]=b'MZ';struct.pack_into('<I',header,60,128)
        self.binary.write_bytes(header+b'PE\0\0'+struct.pack('<H',0x8664))
        self.details['buildTarget']='x86_64-pc-windows-msvc';self.metadata.write_text(json.dumps(self.details))
        with patch('platform.system',return_value='Windows'),patch('subprocess.run',side_effect=self.command):manifest=self.package('release')
        self.assertEqual(manifest['binary'],'dt-cli.exe');self.assertEqual(manifest['nativeProbe'],'performed')
    def test_dirty_or_local_development_release_preserves_previous_package(self):
        self.output.write_bytes(b'previous')
        for field,value in [('buildCommit','b'*40+'+dirty'),('localDevelopment',True),('buildCommit','unknown')]:
            with self.subTest(field=field,value=value):
                original=self.details[field];self.details[field]=value;self.metadata.write_text(json.dumps(self.details))
                with patch('platform.system',return_value='Darwin'),patch('subprocess.run',side_effect=self.command):
                    with self.assertRaises(SystemExit):self.package('release')
                self.assertEqual(self.output.read_bytes(),b'previous');self.details[field]=original
    def test_stale_metadata_and_wrong_artifact_architecture_are_rejected(self):
        self.details['catalogDigest']='d'*64
        with patch('platform.system',return_value='Darwin'),patch('subprocess.run',side_effect=self.command):
            with self.assertRaises(SystemExit):self.package('release')
        self.binary.write_bytes(b'not-macho')
        with self.assertRaises(SystemExit):self.package('candidate')
    def test_candidate_reproducible_and_corrupt_metadata_keeps_prior_package(self):
        self.package('candidate');first=self.output.read_bytes();self.package('candidate');self.assertEqual(self.output.read_bytes(),first)
        self.metadata.write_text('{}')
        with self.assertRaises(SystemExit):self.package('candidate')
        self.assertEqual(self.output.read_bytes(),first)
    def test_windows_pe_machine_is_checked_on_any_build_host(self):
        header=bytearray(128);header[:2]=b'MZ';struct.pack_into('<I',header,60,128)
        for machine,expected in [(0x8664,'x86_64'),(0xAA64,'arm64')]:
            self.binary.write_bytes(header+b'PE\0\0'+struct.pack('<H',machine));self.assertEqual(PACKAGE['binary_architecture'](self.binary,'Windows'),expected)
        self.binary.write_bytes(b'invalid')
        with self.assertRaises(SystemExit):PACKAGE['binary_architecture'](self.binary,'Windows')
if __name__=='__main__':unittest.main()
