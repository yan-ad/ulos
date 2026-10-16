"""Real localhost wire transfers; never fetches trackers or uses DHT."""
import hashlib
import json
import os
from pathlib import Path
import queue
import subprocess
import sys
import tempfile
import threading
import time

worker, fixture = map(os.path.abspath, sys.argv[1:])
# Minimal fixture-only bencode codec: preserve bytes and canonical key ordering.
def decode(data):
    def item(pos):
        c = data[pos:pos+1]
        if c == b'i':
            end = data.index(b'e', pos)
            return int(data[pos+1:end]), end+1
        if c in (b'd', b'l'):
            value = {} if c == b'd' else []
            pos += 1
            while data[pos:pos+1] != b'e':
                v, pos = item(pos)
                if c == b'd':
                    other, pos = item(pos)
                    value[v] = other
                else: value.append(v)
            return value, pos+1
        end = data.index(b':', pos)
        size = int(data[pos:end]); pos = end+1
        return data[pos:pos+size], pos+size
    result, end = item(0)
    assert end == len(data)
    return result

def encode(value):
    if isinstance(value, int): return b'i'+str(value).encode()+b'e'
    if isinstance(value, bytes): return str(len(value)).encode()+b':'+value
    if isinstance(value, list): return b'l'+b''.join(map(encode, value))+b'e'
    return b'd'+b''.join(encode(k)+encode(value[k]) for k in sorted(value))+b'e'
class Events(queue.Queue):
    """Keep child failures actionable without exposing diagnostics in production."""
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
            self.failure('invalid JSON event: '+str(event))
        self.history.append(event)
        if event.get('event') == 'stats':
            for key in ['download_bytes_per_second', 'upload_bytes_per_second',
                        'downloaded_bytes', 'uploaded_bytes', 'total_bytes',
                        'peers', 'connecting_peers', 'queued_peers', 'seen_peers',
                        'stalled_for_secs', 'tracker_count']:
                assert isinstance(event[key], int) and event[key] >= 0, event
            assert event['downloaded_bytes'] <= event['total_bytes'], event
        return event

    def failure(self, reason):
        p = self.process
        try:
            code = p.wait(timeout=2)
        except subprocess.TimeoutExpired:
            code = 'still running'
        stderr = p.stderr.read() if code != 'still running' else '(child still running)'
        raise AssertionError(f'{reason}; command={p.args!r}; exit={code}; stderr={stderr!r}; events={self.history!r}')


def events(p):
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
    return q

def send(p, **kw):
    p.stdin.write(json.dumps(kw) + '\n')
    p.stdin.flush()

def launch(meta, out, upload='1'):
    p = subprocess.Popen([worker, '--metainfo', str(meta), '--output', str(out),
        '--dht', '0', '--upload', upload, '--download-limit', '0',
        '--upload-limit', '0', '--peer-limit', '20'], stdin=subprocess.PIPE,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, encoding='utf-8')
    q = events(p)
    ready = q.get(timeout=5)
    assert ready['event'] == 'ready'
    assert ready['capabilities']['no_upload'] is False
    manifest = q.get(timeout=5)
    assert manifest['event'] == 'manifest'
    # Manifest/start barrier: no session stats or filesystem mutations yet.
    time.sleep(.2)
    assert q.empty() and not list(out.iterdir())
    return p, q, manifest

with tempfile.TemporaryDirectory() as root:
    root = Path(root)/'unicode-文件-é'
    root.mkdir()
    data = root/'source'/'payload'
    data.mkdir(parents=True)
    (data/'a.bin').write_bytes(os.urandom(512*1024+13))
    (data/'文件-é.bin').write_bytes(os.urandom(32769))
    for mode in ['v2', 'hybrid', 'v1']:
        meta = root/(mode+'.torrent')
        subprocess.run([fixture, '--create-fixture', str(data), '--torrent-out', str(meta), '--'+mode], check=True)
        out = root/('out-'+mode)
        out.mkdir()
        seed = subprocess.Popen([fixture, '--seed-test', str(meta), '--data', str(data.parent), '--listen', '127.0.0.1:0'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, encoding='utf-8')
        sq = events(seed)
        try:
            port = sq.get(timeout=35)['port']
            p, q, manifest = launch(meta, out)
            try:
                assert ('infohash_v2' in manifest) == (mode != 'v1')
                assert ('infohash_v1' in manifest) == (mode != 'v2')
                assert len(manifest['files']) == 2
                assert {f['path'] for f in manifest['files']} == {'payload/a.bin', 'payload/文件-é.bin'}
                # A single write exercises queued commands following start.
                p.stdin.write('{"command":"start"}\n{"command":"pause"}\n'); p.stdin.flush()
                while True:
                    e = q.get(timeout=10)
                    assert e and e['event'] != 'error', e
                    if e.get('phase') == 'paused':
                        assert e['download_bytes_per_second'] == e['upload_bytes_per_second'] == 0
                        assert e['stalled_for_secs'] == 0
                        break
                send(p, command='limits', download_bytes_per_second=1048576, upload_bytes_per_second=1048576)
                send(p, command='peers', limit=10)
                send(p, command='resume')
                # Give libtorrent time to finish its initial check before connecting.
                time.sleep(.5)
                send(p, command='connect_peer', address='127.0.0.1', port=port)
                end = time.monotonic()+45
                while time.monotonic()<end:
                    e = q.get(timeout=10)
                    assert e and e['event'] != 'error', e
                    if e['event']=='complete': break
                else: raise AssertionError('transfer timeout')
                assert p.wait(timeout=10)==0
                assert not p.stderr.read()
                for name in ['a.bin', '文件-é.bin']:
                    assert hashlib.sha256((out/'payload'/name).read_bytes()).digest() == hashlib.sha256((data/name).read_bytes()).digest()
            finally:
                if p.poll() is None: p.kill(); p.wait()
        finally:
            seed.stdin.close()
            seed.wait(timeout=10)
    meta = root/'v2.torrent'
    private = decode(meta.read_bytes())
    private[b'info'][b'private'] = 1
    private_meta = root/'private.torrent'
    private_meta.write_bytes(encode(private))
    out = root/'private-out'; out.mkdir()
    p,q,manifest = launch(private_meta,out)
    assert manifest['private'] is True
    p.stdin.close()
    assert q.get(timeout=5)['event'] == 'cancelled'
    assert p.wait(timeout=10) == 0 and not p.stderr.read()

    invalid = root/'invalid.torrent'; invalid.write_bytes(b'not metainfo')
    invalid_out = root/'invalid-out'; invalid_out.mkdir()
    bad = subprocess.run([worker, '--metainfo', str(invalid), '--output', str(invalid_out),
        '--dht', '0', '--upload', '1', '--download-limit', '0', '--upload-limit', '0',
        '--peer-limit', '20'], input='', capture_output=True, text=True, timeout=10)
    assert bad.returncode == 1 and not bad.stderr
    assert json.loads(bad.stdout)['event'] == 'error' and not list(invalid_out.iterdir())
    for action in ['cancel', 'eof', 'running-eof', 'unknown', 'no-upload', 'oversized', 'negative-limit', 'bad-json', 'zero-peers', 'huge-peers', 'partial-eof', 'flood']:
        out=root/action; out.mkdir()
        p,q,_=launch(meta,out,'0' if action=='no-upload' else '1')
        try:
            if action=='eof': p.stdin.close()
            else:
                send(p,command='start')
                if action=='running-eof':
                    p.stdin.close()
                elif action=='partial-eof':
                    p.stdin.write('{'); p.stdin.close()
                elif action=='flood':
                    p.stdin.write('{"command":"pause"}\n'*65); p.stdin.flush()
                elif action in ['zero-peers', 'huge-peers']:
                    send(p, command='peers', limit=0 if action=='zero-peers' else 10001)
                elif action=='oversized':
                    p.stdin.write('x'*1025+'\n'); p.stdin.flush()
                elif action=='bad-json':
                    p.stdin.write('{broken}\n'); p.stdin.flush()
                elif action=='negative-limit':
                    send(p,command='limits',download_bytes_per_second=-1,upload_bytes_per_second=0)
                elif action!='no-upload': send(p,command='cancel' if action=='cancel' else 'unknown')
            expected='cancelled' if action in ['cancel','eof','running-eof'] else 'error'
            while True:
                e=q.get(timeout=10)
                assert e is not None
                if e['event']==expected:break
            assert p.wait(timeout=10)==(1 if expected=='error' else 0)
            assert not p.stderr.read()
        finally:
            if p.poll() is None:p.kill();p.wait()
print('v2/hybrid/v1 localhost transfers, pause/resume/limits, barrier, EOF, cancel and fail-closed checks passed')
