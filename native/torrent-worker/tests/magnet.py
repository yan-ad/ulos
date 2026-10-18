"""Metadata-only BEP 9 exchange with real local v1/v2/hybrid seeders."""
import json
import os
from pathlib import Path
import queue
import subprocess
import sys
import tempfile
import threading

worker, fixture = map(os.path.abspath, sys.argv[1:])

class Events(queue.Queue):
    def __init__(self, process):
        super().__init__()
        self.process = process
        self.history = []

    def get(self, *args, **kwargs):
        try:
            event = super().get(*args, **kwargs)
        except queue.Empty:
            self.failure('timed out waiting for JSON event')
        if event is None:
            self.failure('early EOF on JSON event stream')
        if isinstance(event, Exception):
            self.failure('invalid JSON event: ' + str(event))
        self.history.append(event)
        return event

    def failure(self, reason):
        try:
            code = self.process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            code = 'still running'
        stderr = self.process.stderr.read() if code != 'still running' else '(child still running)'
        raise AssertionError(f'{reason}; command={self.process.args!r}; exit={code}; stderr={stderr!r}; events={self.history!r}')

def spawn(args):
    p = subprocess.Popen(args, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                         stderr=subprocess.PIPE, text=True, encoding='utf-8')
    q = Events(p)
    def read():
        try:
            for line in p.stdout:
                q.put(json.loads(line))
        except Exception as error:
            q.put(error)
        finally:
            q.put(None)
    threading.Thread(target=read, daemon=True).start()
    return p, q

def args(uri, out, timeout='15'):
    return [worker, '--resolve-magnet', uri, '--output', str(out),
            '--dht', '0', '--timeout', timeout]

def finish(p):
    if p.poll() is None:
        p.kill()
    p.wait(timeout=10)

with tempfile.TemporaryDirectory() as tmp:
    root = Path(tmp) / 'unicode-文件-é'
    root.mkdir()
    for size in [1024, 65537]:
        data = root / ('source-' + str(size)) / 'payload.bin'
        data.parent.mkdir()
        data.write_bytes(os.urandom(size))
        for mode in ['v1', 'v2', 'hybrid']:
            meta = root / (str(size) + mode + '.torrent')
            subprocess.run([fixture, '--create-fixture', str(data), '--torrent-out',
                            str(meta), '--' + mode], check=True)
            uri = json.loads(subprocess.check_output([fixture, '--magnet-info', str(meta)]))['magnet']
            assert 'urn:btih:' in uri if mode != 'v2' else 'urn:btmh:' in uri
            if mode == 'hybrid':
                assert 'urn:btmh:' in uri
            seed, sq = spawn([fixture, '--seed-test', str(meta), '--data', str(data.parent),
                              '--listen', '127.0.0.1:0'])
            try:
                port = sq.get(timeout=35)['port']
                out = root / ('out-' + str(size) + mode)
                out.mkdir()
                p, q = spawn(args(uri + '&x.pe=127.0.0.1%3A' + str(port), out))
                try:
                    assert q.get(timeout=5)['event'] == 'ready'
                    terminal = q.get(timeout=25)
                    # BEP 9 alone does not provide multi-piece BEP 52 layers.
                    # Refuse incomplete metainfo rather than fabricate proof data.
                    expected = 'error' if size > 16384 and mode != 'v1' else 'metadata'
                    assert terminal['event'] == expected, (mode, size, terminal)
                    assert p.wait(timeout=10) == (1 if expected == 'error' else 0)
                    assert not p.stderr.read()
                    assert not list(out.iterdir()), list(out.rglob('*'))
                    if expected == 'metadata':
                        value = terminal['metainfo_hex']
                        assert value == value.lower() and len(value) <= 16 * 1024 * 1024
                        encoded = bytes.fromhex(value)
                        assert encoded.startswith(b'd') and b'4:info' in encoded
                        # Load through the real worker parser and compare its hashes
                        # to the original fixture's magnet identities.
                        resolved = root / ('resolved-' + str(size) + mode + '.torrent')
                        resolved.write_bytes(encoded)
                        roundtrip = json.loads(subprocess.check_output(
                            [fixture, '--magnet-info', str(resolved)]))['magnet']
                        from urllib.parse import parse_qs, urlsplit
                        assert parse_qs(urlsplit(roundtrip).query)['xt'] == parse_qs(urlsplit(uri).query)['xt']
                finally:
                    finish(p)
            finally:
                seed.stdin.close()
                seed.wait(timeout=10)
    uri = 'magnet:?xt=urn:btih:' + '01' * 20
    for action in ['cancel', 'eof', 'timeout', 'unknown']:
        out = root / action
        out.mkdir()
        p, q = spawn(args(uri, out, '1' if action == 'timeout' else '15'))
        try:
            ready = q.get(timeout=5)
            assert ready['event'] == 'ready' and ready['protocol'] == 1 and ready['engine'] == 'libtorrent'
            if action == 'eof':
                p.stdin.close()
            elif action != 'timeout':
                p.stdin.write(json.dumps({'command': action}) + '\n')
                p.stdin.flush()
            expected = 'cancelled' if action in ['cancel', 'eof'] else 'error'
            assert q.get(timeout=8)['event'] == expected
            assert p.wait(timeout=10) == (0 if expected == 'cancelled' else 1)
            assert not list(out.iterdir()) and not p.stderr.read()
        finally:
            finish(p)
    for bad_uri, timeout in [('magnet:?xt=no', '2'), (uri, '0'), (uri, '121'),
                             (uri + '&dn=' + 'x' * 16384, '2'),
                             (uri + '&ws=http%3A%2F%2F127.0.0.1%3A1%2Fseed', '2')]:
        out = root / 'bad'
        out.mkdir(exist_ok=True)
        result = subprocess.run(args(bad_uri, out, timeout), input='', capture_output=True,
                                text=True, timeout=10)
        assert result.returncode == 1 and not result.stderr
        assert json.loads(result.stdout)['event'] == 'error' and not list(out.iterdir())
print('v1/v2/hybrid metadata resolution, complete-infohash roundtrip, no payload writes, cancel/EOF/timeout/invalid checks passed; multi-piece v2/hybrid correctly fail closed for missing BEP 52 layers')
