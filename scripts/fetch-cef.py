#!/usr/bin/env python3
"""Fetch only the pinned official CEF distribution, validate its digest, extract safely."""
import hashlib, json, pathlib, tarfile, tempfile, urllib.request, shutil
root=pathlib.Path(__file__).resolve().parent.parent
pin=json.loads((root/'cef-version.json').read_text())
assert pin['source'].startswith('https://cef-builds.spotifycdn.com/')
target=root/'vendor'/'cef'
if target.exists():
    version=(target/'include'/'cef_version.h').read_text()
    if pin['cef_version'] not in version: raise SystemExit('Existing CEF version differs from pin; remove vendor/cef explicitly')
    print('Pinned CEF already materialized');raise SystemExit(0)
target.parent.mkdir(exist_ok=True)
with tempfile.TemporaryDirectory(dir=target.parent) as tmp:
    tmp=pathlib.Path(tmp);archive=tmp/'cef.tar.bz2'
    with urllib.request.urlopen(pin['source'], timeout=120) as response, archive.open('wb') as out: shutil.copyfileobj(response,out)
    data=archive.read_bytes()
    if len(data)!=pin['size'] or hashlib.sha1(data).hexdigest()!=pin['sha1'] or hashlib.sha256(data).hexdigest()!=pin['sha256']: raise SystemExit('CEF artifact digest or size mismatch')
    print('Official SHA-1 verified; SHA-256:',hashlib.sha256(data).hexdigest())
    with tarfile.open(archive,'r:bz2') as tar: tar.extractall(tmp,filter='data')
    directories=[p for p in tmp.iterdir() if p.is_dir()]
    if len(directories)!=1: raise SystemExit('Unexpected CEF archive layout')
    directories[0].rename(target)
