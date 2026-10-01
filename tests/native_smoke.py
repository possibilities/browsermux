#!/usr/bin/env python3
"""Run real AppKit/CEF bundle with same-origin A/B/temporary storage fixtures."""
import http.server, json, os, pathlib, subprocess, tempfile, threading, time
ROOT=pathlib.Path(__file__).resolve().parent.parent
results=[]
class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self,*args):pass
    def do_GET(self):
        if self.path=='/sw.js':
            content=b"self.addEventListener('install',e=>self.skipWaiting());self.addEventListener('activate',e=>e.waitUntil(self.clients.claim()));";kind='application/javascript'
        else:content=(ROOT/'tests/fixtures/storage.html').read_bytes();kind='text/html'
        self.send_response(200);self.send_header('Content-Type',kind);self.send_header('Cache-Control','no-store');self.end_headers();self.wfile.write(content)
    def do_POST(self):
        if self.path!='/result':self.send_error(404);return
        length=int(self.headers.get('Content-Length','0'))
        if not 0<length<16384:self.send_error(413);return
        results.append(json.loads(self.rfile.read(length)));self.send_response(204);self.end_headers()
server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler)
threading.Thread(target=server.serve_forever,daemon=True).start()
(ROOT/'build').mkdir(exist_ok=True)
with tempfile.TemporaryDirectory(prefix='pane-native-smoke-') as data:
    env=dict(os.environ,SPB_DATA_ROOT=data,SPB_FIXTURE_URL=f'http://127.0.0.1:{server.server_port}/')
    proc=subprocess.Popen([str(ROOT/'build/browsermux.app/Contents/MacOS/browsermux'),'--smoke-test'],env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True)
    try:log,_=proc.communicate(timeout=90)
    except subprocess.TimeoutExpired:proc.kill();log,_=proc.communicate();log+='\nTIMEOUT\n'
    (ROOT/'build/native-smoke.log').write_text(log)
    (ROOT/'build/native-smoke-results.json').write_text(json.dumps({'exit_code':proc.returncode,'results':results},indent=2))
    print(log)
    print(json.dumps(results,indent=2))
    assert proc.returncode==0,'Native process failed'
    assert 'NATIVE_SMOKE panes=4 browsers=4 sandbox_required=true' in log,'Native pane lifecycle check missing'
    assert len(results)==4,f'Expected four fixture results, received {len(results)}'
    assert all(r.get('pass') for r in results),'Browser storage leaked or failed'
    assert {r['expected'] for r in results}=={'A','B','T'},'Missing profile coverage'
    assert any(r['expected']=='A' and r['mode']=='read' for r in results),'Same-profile sharing not tested'
    # Sanitized Rust metadata must not contain temporary fixture URLs or HTTP query tokens.
    import sqlite3
    for db in pathlib.Path(data).glob('*.sqlite*'):
        if db.suffix not in {'.sqlite','.sqlite3'}:continue
        con=sqlite3.connect(db)
        for table, in con.execute("SELECT name FROM sqlite_master WHERE type='table'"):
            for row in con.execute('SELECT * FROM "'+table.replace('"','""')+'"'):
                assert '?expected=' not in str(row),'Unsafe fixture query URL persisted'
        con.close()
server.shutdown()
print('PASS: real CEF native panes, same-container sharing and isolated cookies/localStorage/IndexedDB/cache')
