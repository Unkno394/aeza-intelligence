#!/usr/bin/env python3
import argparse,hashlib,json,os,socket,subprocess,tempfile,time,urllib.request
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--engine',required=True);p.add_argument('--snapshot',required=True);p.add_argument('--output',required=True);a=p.parse_args()
with tempfile.TemporaryDirectory(prefix='aeza-market-refresh-') as tmp:
 path=Path(tmp)/'market.json';original=Path(a.snapshot).read_bytes();path.write_bytes(original)
 with socket.socket() as sock:sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
 proc=subprocess.Popen([a.engine],env={**os.environ,'MARKET_DATA_PATH':str(path),'MARKET_BIND':f'127.0.0.1:{port}'},stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
 def get():
  with urllib.request.urlopen(f'http://127.0.0.1:{port}/api/market/v2/sources',timeout=5) as r:return json.load(r)
 try:
  for _ in range(100):
   try:first=get();break
   except Exception:time.sleep(.1)
  else:raise AssertionError('server did not start')
  envelope=json.loads(original);data=json.loads(envelope['payload']);data['catalog_version']='runtime-test-version-only';payload=json.dumps(data,separators=(',',':'));new=json.dumps({'payload':payload,'sha256':hashlib.sha256(payload.encode()).hexdigest()});pending=path.with_suffix('.pending');pending.write_text(new);os.replace(pending,path);second=get();assert second['catalog_version']=='runtime-test-version-only'
  pending.write_text('{broken');os.replace(pending,path);fallback=get();assert fallback['catalog_version']=='runtime-test-version-only';assert fallback['snapshot_status'].startswith('last_verified_snapshot:')
  pending.write_bytes(original);os.replace(pending,path);restored=get();assert restored['catalog_version']==first['catalog_version'];assert restored['snapshot_status']=='verified_runtime_snapshot'
  Path(a.output).write_text(json.dumps({'passed':True,'expected':'same running process loads atomic verified update; bad JSON retains last verified; restoration succeeds','initial':first['catalog_version'],'updated':second['catalog_version'],'fallback_status':fallback['snapshot_status'],'restored':restored['catalog_version']},indent=2));print('PASS real HTTP update, fallback, restoration without restart')
 finally:proc.terminate();proc.wait(timeout=5)
