#!/usr/bin/env python3
"""Fetch public facts and atomically publish a verified snapshot; failure preserves previous file.
No credentials, cloud resources or private account APIs. Python standard library only.
"""
import argparse,datetime,hashlib,json,re,os,tempfile,time,urllib.request,urllib.parse,xml.etree.ElementTree as ET
from decimal import Decimal,ROUND_HALF_EVEN
from pathlib import Path
BASE='https://cloudfinops.ru/api/v1'; AEZA='https://aeza.net/ru/virtual-servers'; CBR='https://www.cbr.ru/scripts/XML_daily.asp'
D=Decimal

def fetch(url,body=None):
    req=urllib.request.Request(url,data=None if body is None else json.dumps(body).encode(),headers={'Content-Type':'application/json','User-Agent':'AezaIntelligence-MarketSimulator/1.3'})
    with urllib.request.urlopen(req,timeout=40) as r:return r.read()
def number(x):return format(D(str(x)),'f')
def convert(amount,currency,fx):
    v=D(amount)*D(fx['rub_per_unit'][currency])/D(fx['rub_per_unit']['USD'])
    return format(v.quantize(D('.000000001'),rounding=ROUND_HALF_EVEN),'f')
def original(amount,currency,unit):return {'amount':number(amount),'currency':currency,'unit':unit}
def region(s):
    if s=='Россия':return 'Russia'
    for prefix,canonical in [('Москва','Moscow'),('Санкт-Петербург','Saint Petersburg')]:
        if s.startswith(prefix):return canonical
    return s

def build(rawdir,legacy):
    rawdir.mkdir(parents=True,exist_ok=True); receipts=[]
    def get(name,url,body=None):
        b=fetch(url,body);(rawdir/name).write_bytes(b);receipts.append({'file':name,'url':url,'method':'POST' if body is not None else 'GET','request':body,'sha256':hashlib.sha256(b).hexdigest(),'fetched_at':datetime.datetime.now(datetime.timezone.utc).isoformat()});return b
    xml=get('cbr.xml',CBR);root=ET.fromstring(xml);date=datetime.datetime.strptime(root.attrib['Date'],'%d.%m.%Y').date()
    if date>datetime.datetime.now().date() or (datetime.datetime.now().date()-date).days>10:raise ValueError('CBR rate future or >10 days stale; retain last verified snapshot')
    fx={'date':date.isoformat(),'source':CBR,'rub_per_unit':{'RUB':'1'},'rounding':'half_even_9_USD_decimals'}
    for v in root:
        code=v.findtext('CharCode')
        if code in ['USD','EUR']:fx['rub_per_unit'][code]=number(D(v.findtext('Value').replace(',','.'))/D(v.findtext('Nominal')))
    if set(fx['rub_per_unit'])!={'RUB','USD','EUR'}:raise ValueError('missing FX rates')
    providers=json.loads(get('providers.json',BASE+'/providers'));products=[];url=BASE+'/products?limit=100';version=None
    for page in range(20):
        response=json.loads(get(f'products-{page}.json',url));current=response['meta']['catalogVersion']
        if version is not None and current!=version:raise ValueError('catalog changed during pagination; rerun')
        version=current;products.extend(response['data']);cursor=response['pagination']['nextCursor']
        if not cursor:break
        url=BASE+'/products?'+urllib.parse.urlencode({'cursor':cursor,'limit':100})
    else:raise ValueError('pagination did not terminate')
    if len({p['id'] for p in products})!=len(products):raise ValueError('duplicate products')
    expected=sum(p['productCount'] for p in providers['data'])
    if len(products)!=expected:raise ValueError(f'incomplete SKU inventory {len(products)} vs {expected}')
    productmap={p['id']:p for p in products};offers=[];excluded=[]
    for p in json.loads(legacy.read_text()):
        p['details']={'original_price':original(p['monthly_usd'],'USD','month'),'normalized_monthly_usd':number(p['monthly_usd']),'fx':None,'market':'international' if p['regions'] else 'unknown','service_type':'vps','virtualization':None,'cpu_class':None,'vat':'unknown','complete_monthly_payment':False,'availability':'unknown','billing_hours':None,'hourly_price':None,'components':[],'source_region':', '.join(p['regions']),'notes':['Legacy VPS Price Index: taxes, virtualization, IP and GB/GiB convention not verified.'],'egress_gib':None}
        offers.append(p)
    for cpu,ram,disk in [(1,2,30),(2,4,60),(4,8,120)]:
        req={'resource':{'type':'compute','vcpu':cpu,'memoryGiB':ram,'disk':{'sizeGiB':disk,'media':'ssd'},'purchaseModel':'on-demand'},'addons':{'publicIpCount':1,'internetEgressGiB':100},'period':'month'}
        response=json.loads(get(f'estimate-{cpu}-{ram}-{disk}.json',BASE+'/estimates',req))
        if response['meta']['catalogVersion']!=version:raise ValueError('estimate catalog version differs')
        for q in response['data']['quotes']:
            if q['status']!='priced':excluded.append({'provider':q['provider'],'status':q['status'],'reason':q.get('reason'),'request':req});continue
            m=q['matchedResource'];total=q['total'];items=[];dates=[]
            for item in q['lineItems']:
                product=productmap[item['productId']];price=next(p for p in product['prices'] if p['id']==item['priceId']);dates.append(price['checkedAt']);items.append({**item,'original_unit_price':price,'source':product['source'],'source_region':product['region']})
            if any(x['vat']!='included' or x['amount']['currency']!='RUB' for x in items):raise ValueError('quote claims priced without billable VAT/currency')
            if abs(sum(D(x['amount']['amount']) for x in items)-D(total['amount']))>D('.01'):raise ValueError('component sum differs from total')
            amount=total['amount'];usd=convert(amount,total['currency'],fx);provider=q['provider'];reg=m['region']
            # GiB is converted to GB; we never silently treat it as the same unit.
            offers.append({'id':f'finops-{provider["id"]}-{cpu}-{ram}-{disk}','provider':provider['name'],'name':f'Cloud VM {m["vcpu"]}vCPU / {m["memoryGiB"]}GiB / {m["disk"]["sizeGiB"]}GiB SSD + IPv4 + 100GiB egress','vcpu':m['vcpu'],'ram_gb':float(D(str(m['memoryGiB']))*D('1.073741824')),'disk_gb':float(D(str(m['disk']['sizeGiB']))*D('1.073741824')),'monthly_usd':float(usd),'regions':[region(reg)],'origin':'observed_component_estimate','source':BASE+'/estimates','observed_at':min(dates),'details':{'original_price':original(amount,total['currency'],'month'),'normalized_monthly_usd':usd,'fx':fx,'market':'russia','service_type':'cloud_vm','virtualization':None,'cpu_class':'100_percent_catalog_compute','vat':'included','complete_monthly_payment':True,'availability':'unknown','billing_hours':720,'hourly_price':None,'components':items,'source_region':reg,'notes':['Derived configuration from actual billable SKUs, not a published fixed VPS plan.','Virtualization is not disclosed in this quote; excluded from verified direct comparison.','720-hour month; one public IPv4 and 100GiB egress priced; no managed services or Windows license.','Resource GiB converted to GB ×1.073741824; retained original matchedResource in component snapshot.'],'egress_gib':100}})
    html=get('aeza.html',AEZA).decode();nextdata=json.loads(re.search(r'<script id="__NEXT_DATA__" type="application/json">(.*?)</script>',html,re.S)[1]);props=nextdata['props']['pageProps']
    # Cents convention checked against all five visible SSR prices and common UI currency.
    if '€' not in html or 'KVM' not in json.dumps(props,ensure_ascii=False):raise ValueError('cannot verify Aéza currency or hypervisor')
    for plan in props['plans']['77']:
        value=f'{D(plan["prices"]["base"]["month"])/100:.2f}'
        if value not in html:raise ValueError('Aéza cents do not reconcile with rendered EUR price')
    locations={str(l['id']):l for l in props['locations']};aeza=[]
    cities={'Варшава':'Warsaw','Москва':'Moscow','Франкфурт':'Frankfurt','Париж':'Paris','Амстердам':'Amsterdam','Шарлотт':'Charlotte','Стокгольм':'Stockholm','Вена':'Vienna','Лондон':'London','Хельсинки':'Helsinki','Санкт-Петербург':'Saint Petersburg'}
    for lid,groups in props['groups'].items():
        loc=locations[lid]
        for group in groups:
            if group['payload'].get('mode') not in ['shared','dedicated']:continue # promotions separate, never generic baseline
            for p in props['plans'].get(str(group['id']),[]):
                if not p['isPublic']:continue
                spec=p['parameters'];base=p['prices']['base'];amount=number(D(base['month'])/100);usd=convert(amount,'EUR',fx)
                active=loc['isAvailable'] and group['isAvailable'] and p['isAvailable'] and not group['payload'].get('isDisabled',False)
                aeza.append({'id':f'aeza-official-{p["id"]}','provider':'aeza','name':p['name'],'vcpu':spec['cpu'],'ram_gb':spec['ram'],'disk_gb':spec['rom'],'monthly_usd':float(usd),'regions':[cities[loc['name']]],'origin':'observed_list_price','source':AEZA,'observed_at':datetime.datetime.now(datetime.timezone.utc).date().isoformat(),'details':{'original_price':original(amount,'EUR','month'),'normalized_monthly_usd':usd,'fx':fx,'market':'russia' if loc['payload']['code']=='ru' else 'international','service_type':'vps','virtualization':'kvm','cpu_class':group['payload']['mode'],'vat':'unknown','complete_monthly_payment':False,'availability':'available_from_source' if active else 'unavailable','billing_hours':None,'hourly_price':original(D(base['hour'])/100,'EUR','hour') if 'hour' in base else None,'components':[],'source_region':loc['name'],'notes':['Official EUR cents verified against rendered SSR prices. No bonus, cashback, prepaid discount or paid control panel applied.','1 IPv4 and advertised unlimited traffic included; optional software/license excluded. VAT treatment not disclosed on public page.','Availability flag is a snapshot, not a measured pool capacity.'],'egress_gib':None}})
    if not aeza:raise ValueError('no official Aéza plans parsed')
    snapshot={'schema_version':1,'prepared_as_of':datetime.datetime.now(datetime.timezone.utc).isoformat(),'catalog_version':version,'fx':fx,'plans':offers,'aeza_plans':aeza,'providers':providers['data'],'raw_product_count':len(products),'excluded_quotes':excluded,'receipts':receipts,'limitations':['Original 322 VPS offers retained; complete cloud configurations are a separate service class.','Cloud totals include VAT, disk, one IPv4, 100GiB egress and 720h; unknown virtualization is not KVM.','Aéza VAT is unknown; unavailable official plans excluded from sales; capacity remains a scenario assumption.','CBR reference FX is not a payment processor quote. Original source dates differ from retrieval and conversion dates.']}
    for p in offers+aeza:
        meta=p['details'];op=meta['original_price'];meta['normalized_monthly_rub']=format((D(op['amount'])*D(fx['rub_per_unit'][op['currency']])).quantize(D('.000000001'),rounding=ROUND_HALF_EVEN),'f')
        if not all(float(p[k])>0 for k in ['vcpu','ram_gb','disk_gb','monthly_usd']):raise ValueError('missing price/specification')
    return snapshot

def main():
    ap=argparse.ArgumentParser();ap.add_argument('--output',type=Path,default=Path(__file__).resolve().parents[1]/'data/market-snapshot.json');ap.add_argument('--raw-dir',type=Path,required=True);ap.add_argument('--legacy',type=Path,default=Path(__file__).resolve().parents[1]/'data/catalog.json');args=ap.parse_args()
    try:
        data=build(args.raw_dir,args.legacy)
        if args.output.exists():
            old=json.loads(args.output.read_text());assert hashlib.sha256(old['payload'].encode()).hexdigest()==old['sha256'];data['contexts']=json.loads(old['payload']).get('contexts',{})
        payload=json.dumps(data,ensure_ascii=False,separators=(',',':'));envelope=json.dumps({'payload':payload,'sha256':hashlib.sha256(payload.encode()).hexdigest()},ensure_ascii=False)
        args.output.parent.mkdir(parents=True,exist_ok=True)
        with tempfile.NamedTemporaryFile('w',encoding='utf-8',dir=args.output.parent,delete=False) as f:f.write(envelope);f.flush();os.fsync(f.fileno());tmp=f.name
        os.replace(tmp,args.output)
        print(json.dumps({'published':str(args.output),'competitor_offers':len(data['plans']),'official_aeza':len(data['aeza_plans']),'finops_skus':data['raw_product_count'],'fx':data['fx']},ensure_ascii=False,indent=2))
    except Exception as e:print('Refresh failed; last verified snapshot retained:',e);raise SystemExit(1)
if __name__=='__main__':main()
