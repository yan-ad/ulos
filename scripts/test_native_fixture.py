"""Fixture regressions; CTest supplies native binaries for real Unicode smoke tests."""
import ast
import io
import json
import os
from pathlib import Path
import queue
import subprocess
import sys
import tempfile
import threading
import unittest
from unittest import mock
from urllib.parse import parse_qs, urlsplit

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = None
WORKER = None


def event_reader(name):
    # Load only the reader definitions: importing a wire-test script runs it.
    path = ROOT / 'native/torrent-worker/tests' / name
    tree = ast.parse(path.read_text(encoding='utf-8'))
    definitions = [node for node in tree.body if isinstance(node, (ast.ClassDef, ast.FunctionDef))
                   and node.name in ('Events', 'events', 'spawn')]
    namespace = dict(queue=queue, subprocess=subprocess, json=json, threading=threading)
    exec(compile(ast.Module(body=definitions, type_ignores=[]), str(path), 'exec'), namespace)
    return namespace


class EventReaderTests(unittest.TestCase):
    def test_child_failure_is_not_a_none_subscript(self):
        for name in ('loopback.py', 'magnet.py'):
            with self.subTest(script=name):
                child = mock.Mock(args=['fixture', '--seed-test'], stderr=io.StringIO('Fixture operation failed\n'))
                child.wait.return_value = 1
                events = event_reader(name)['Events'](child)
                events.put({'event': 'prior'})
                self.assertEqual(events.get()['event'], 'prior')
                events.put(None)
                with self.assertRaisesRegex(AssertionError, 'early EOF.*exit=1.*Fixture operation failed.*prior'):
                    events.get(timeout=.1)

    def test_invalid_json_and_timeout_are_actionable(self):
        for name in ('loopback.py', 'magnet.py'):
            with self.subTest(script=name):
                child = mock.Mock(args=['fixture'], stderr=io.StringIO(''))
                child.wait.return_value = 1
                events = event_reader(name)['Events'](child)
                events.put(ValueError('bad JSON'))
                with self.assertRaisesRegex(AssertionError, 'invalid JSON event: bad JSON'):
                    events.get(timeout=.1)
                with self.assertRaisesRegex(AssertionError, 'timed out waiting for JSON event'):
                    events.get(timeout=.01)

    def test_magnet_reader_reports_parse_error_and_eof(self):
        definitions = event_reader('magnet.py')
        child = mock.Mock(stdout=io.StringIO('{invalid}\n'), stderr=io.StringIO(''), args=['fixture'])
        child.wait.return_value = 1
        with mock.patch.object(subprocess, 'Popen', return_value=child):
            _, events = definitions['spawn'](['fixture'])
            with self.assertRaisesRegex(AssertionError, 'invalid JSON event'):
                events.get(timeout=2)
            with self.assertRaisesRegex(AssertionError, 'early EOF'):
                events.get(timeout=2)


@unittest.skipUnless(FIXTURE, 'native binaries supplied by CTest or --fixture/--worker')
class NativeFixtureTests(unittest.TestCase):
    def run_fixture(self, *args, **kwargs):
        return subprocess.run([FIXTURE, *map(str, args)], input='\n', capture_output=True,
                              text=True, encoding='utf-8', timeout=35, **kwargs)

    def test_unicode_metainfo_magnet_and_seed_roundtrip(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / 'unicode-文件-é'
            root.mkdir()
            data = root / 'source' / '文件-é.bin'
            data.parent.mkdir()
            data.write_bytes(bytes(range(256)) * 4)
            for mode in ('v1', 'v2', 'hybrid'):
                with self.subTest(mode=mode):
                    meta = root / (mode + '-文件-é.torrent')
                    created = self.run_fixture('--create-fixture', data, '--torrent-out', meta, '--' + mode)
                    self.assertEqual(created.returncode, 0, created.stderr)
                    self.assertTrue(meta.read_bytes().startswith(b'd'))
                    magnet = self.run_fixture('--magnet-info', meta)
                    self.assertEqual(magnet.returncode, 0, magnet.stderr)
                    uri = json.loads(magnet.stdout)['magnet']
                    query = parse_qs(urlsplit(uri).query)
                    self.assertEqual(any(v.startswith('urn:btih:') for v in query['xt']), mode != 'v2')
                    self.assertEqual(any(v.startswith('urn:btmh:') for v in query['xt']), mode != 'v1')
                    ascii_meta = Path(tmp) / (mode + '.torrent')
                    ascii_meta.write_bytes(meta.read_bytes())
                    ascii_magnet = self.run_fixture('--magnet-info', ascii_meta)
                    self.assertEqual(ascii_magnet.returncode, 0, ascii_magnet.stderr)
                    self.assertEqual(json.loads(ascii_magnet.stdout), json.loads(magnet.stdout))
                    seed = self.run_fixture('--seed-test', meta, '--data', data.parent, '--listen', '127.0.0.1:0')
                    self.assertEqual(seed.returncode, 0, seed.stderr)
                    ready = json.loads(seed.stdout)
                    self.assertEqual(ready['event'], 'seed_ready')
                    self.assertGreater(ready['port'], 0)
                    out = root / ('out-' + mode)
                    out.mkdir()
                    worker = subprocess.run([WORKER, '--metainfo', str(meta), '--output', str(out),
                        '--dht', '0', '--upload', '1', '--download-limit', '0', '--upload-limit', '0',
                        '--peer-limit', '20'], input='', capture_output=True, text=True, encoding='utf-8', timeout=10)
                    self.assertEqual(worker.returncode, 0, worker.stderr)
                    events = list(map(json.loads, worker.stdout.splitlines()))
                    self.assertEqual([e['event'] for e in events], ['ready', 'manifest', 'cancelled'])
                    self.assertEqual(events[1]['files'][0]['path'], '文件-é.bin')
                    self.assertEqual(list(out.iterdir()), [])

    def test_missing_empty_invalid_and_oversized_metainfo(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / 'unicode-文件-é'
            root.mkdir()
            for name, content in (('missing', None), ('empty', b''), ('invalid', b'not metainfo'),
                                  ('oversized', b'x' * (8 * 1024 * 1024 + 1))):
                meta = root / (name + '.torrent')
                if content is not None:
                    meta.write_bytes(content)
                for args in (('--magnet-info', meta),
                             ('--seed-test', meta, '--data', root, '--listen', '127.0.0.1:0')):
                    with self.subTest(case=name, command=args[0]):
                        result = self.run_fixture(*args)
                        self.assertEqual(result.returncode, 1)
                        self.assertEqual(result.stdout, '')
                        self.assertIn('Fixture operation failed', result.stderr)


if __name__ == '__main__':
    if '--fixture' in sys.argv:
        index = sys.argv.index('--fixture')
        FIXTURE = os.path.abspath(sys.argv.pop(index + 1))
        sys.argv.pop(index)
        index = sys.argv.index('--worker')
        WORKER = os.path.abspath(sys.argv.pop(index + 1))
        sys.argv.pop(index)
        # skipUnless is evaluated at class definition, before argument parsing.
        NativeFixtureTests.__unittest_skip__ = False
    unittest.main()
