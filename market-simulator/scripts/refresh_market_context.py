#!/usr/bin/env python3
"""Update separate GPU/Akash research contexts in the atomic market snapshot.
Requires pyarrow (only for GPU parquet). No GPU listing becomes a VPS plan.
"""
import argparse,collections,datetime,hashlib,json,os,tempfile,urllib.request,xml.etree.ElementTree as ET
from decimal import Decimal,ROUND_HALF_EVEN
from pathlib import Path

def get(url,path):
 b=urllib.request.urlopen(urllib.request.Request(url,headers={'User-Agent':'AezaIntelligence/1.3'}),timeout=40).read();path.write_bytes(b);return b

def main():
 p=argparse.ArgumentParser();p.add_argument('--snapshot',type=Path,required=True);p.add_argument('--raw-dir',type=Path,required=True);args=p.parse_args();args.raw_dir.mkdir(parents=True,exist_ok=True)
 env=json.loads(args.snapshot.read_text());assert hashlib.sha256(env['payload'].encode()).hexdigest()==env['sha256'];s=json.loads(env['payload']);contexts=s.get('contexts',{});now=datetime.datetime.now(datetime.timezone.utc).isoformat()
 try:
  import pyarrow.parquet as pq
  listing=get('https://hubbard-gpu-price-data.s3.amazonaws.com/?list-type=2&prefix=prices/&max-keys=1000',args.raw_dir/'gpu-s3-list.xml');root=ET.fromstring(listing);ns={'s':'http://s3.amazonaws.com/doc/2006-03-01/'}
  if root.findtext('s:IsTruncated',namespaces=ns)=='true':raise ValueError('S3 list truncated: do not assume latest key')
  keys=sorted(x.text for x in root.findall('s:Contents/s:Key',ns) if x.text.endswith('.parquet'));url='https://hubbard-gpu-price-data.s3.amazonaws.com/'+keys[-1]
  b=get(url,args.raw_dir/'gpu-latest.parquet');rows=pq.read_table(args.raw_dir/'gpu-latest.parquet').to_pylist();valid=[r for r in rows if r.get('quality')=='ok' and r.get('gpu_count',0)>0 and r.get('price_per_hour',0)>0]
  selected=[];counts=collections.Counter();usd=Decimal(s['fx']['rub_per_unit']['USD'])
  for r in sorted(valid,key=lambda r:(r['provider'],r['is_spot'],r['gpu_type'],r['region'],r['instance_type'])):
   if counts[r['provider']]>=5:continue
   counts[r['provider']]+=1;selected.append({**r,'timestamp':r['timestamp'].isoformat(),'original_price':{'amount':str(r['price_per_hour']),'currency':'USD','unit':'hour'},'price_rub_hour':format((Decimal(str(r['price_per_hour']))*usd).quantize(Decimal('.000000001'),rounding=ROUND_HALF_EVEN),'f'),'source':url,'fx_date':s['fx']['date']})
  contexts['gpu']={'source':'https://github.com/alex-hubbard/gpu_price_tracker','data_source':url,'license':'CC-BY-4.0','fetched_at':now,'observed_rows':len(rows),'quality_ok_rows':len(valid),'listing_sample_count':len(selected),'providers':dict(collections.Counter(r['provider'] for r in valid)),'latest_timestamp':max(r['timestamp'] for r in valid).isoformat(),'sha256':hashlib.sha256(b).hexdigest(),'fx':s['fx'],'listings':selected,'usage':'reference_only_gpu_not_vps','limitations':['Sample up to five observed listings per provider; full raw snapshot retained as Parquet.','Spot and on-demand are separate labels. GPU-only and complete VM billing scopes differ; no monthly VPS price inferred.','Disk, taxes, IPv4 and full VM configuration can be missing. No substitution into ordinary VPS simulation.']}
 except Exception as e:
  if 'gpu' not in contexts:raise
  contexts['gpu']['refresh_error']=str(e)
 try:
  url='https://console-api.akash.network/v1/graph-data/dailyLeaseCount';b=get(url,args.raw_dir/'akash-daily-lease-count.json');d=json.loads(b);rows=d if isinstance(d,list) else d.get('snapshots',d.get('data',[]))
  # actual API records carry date and count, not inferred block UTC or prices
  if isinstance(rows,dict):raise ValueError('unknown Akash graph envelope')
  points=[]
  for r in rows:
   date=r.get('date') or r.get('date_utc');value=r.get('dailyLeaseCount')
   if value is None:value=r.get('value')
   if value is None:value=r.get('count')
   if not date or value is None:raise ValueError('unknown Akash date/count fields')
   count=int(value)
   if count<0:raise ValueError('negative lease count')
   points.append({'date':str(date)[:10],'new_lease_events':count})
  if len(points)<30:raise ValueError('incomplete Akash history')
  points.sort(key=lambda r:r['date']);monthly=collections.Counter()
  today=datetime.datetime.now(datetime.timezone.utc).strftime('%Y-%m')
  for r in points:
   if r['date'][:7]<today:monthly[r['date'][:7]]+=r['new_lease_events']
  months=sorted(monthly)[-6:];mean=sum(monthly[m] for m in months)/len(months)
  contexts['akash']={'source':url,'fetched_at':now,'daily_points':len(points),'first_date':points[0]['date'],'last_date':points[-1]['date'],'sha256':hashlib.sha256(b).hexdigest(),'monthly_profile':{'months':months,'new_lease_events':[monthly[m] for m in months],'normalized_weights':[monthly[m]/mean for m in months]},'recent_daily':points[-60:],'usage':'arrival_shape_only_not_vps_price_or_elasticity','limitations':['Container leases are not VPS. Only relative arrival shape is transferred as a scenario assumption.','No AKT/USD conversion, elasticity, customer retention or Aéza capacity inferred.','Historical joined CSV contains implausible active_provider_count values; that column and monetary joins are not used.']}
 except Exception as e:
  if 'akash' not in contexts:
   profile=json.loads((args.snapshot.parent/'akash_profile.json').read_text());contexts['akash']={'source':'https://console-api.akash.network/v1/graph-data/dailyLeaseCount','refresh_error':str(e),'monthly_profile':profile,'usage':'verified_historical_arrival_shape_only'}
  else:contexts['akash']['refresh_error']=str(e)
 readme=get('https://raw.githubusercontent.com/public-apis/public-apis/master/README.md',args.raw_dir/'public-apis.md')
 contexts['api_discovery']={'source':'https://github.com/public-apis/public-apis','checked_at':now,'sha256':hashlib.sha256(readme).hexdigest(),'selected':'Bank of Russia SXML','additional_candidates':['Frankfurter','National Bank of Poland','Currency-api'],'decision':'Use primary CBR RUB reference rates. Other endpoints are discovery candidates, not invented backup rates.'}
 try:
  get('https://vpslocate.com/data/current.json',args.raw_dir/'vps-current.json');index_status='new_source_downloaded_schema_requires_review_using_verified_archive'
 except Exception as e:index_status='using_verified_archive: '+str(e)
 contexts['vps_price_index']={'source':'https://vpslocate.com/data/','observation_date':'2026-10-23','included_vps':322,'refresh_status':index_status,'notes':'No new prices synthesized; immutable archived observations retained.'}
 s['contexts']=contexts;payload=json.dumps(s,ensure_ascii=False,separators=(',',':'));env={'payload':payload,'sha256':hashlib.sha256(payload.encode()).hexdigest()}
 with tempfile.NamedTemporaryFile('w',encoding='utf-8',dir=args.snapshot.parent,delete=False) as f:json.dump(env,f,ensure_ascii=False);f.flush();os.fsync(f.fileno());tmp=f.name
 os.replace(tmp,args.snapshot);print(json.dumps({k:{x:v for x,v in d.items() if x not in ['listings','recent_daily']} for k,d in contexts.items()},ensure_ascii=False,indent=2))
if __name__=='__main__':main()
