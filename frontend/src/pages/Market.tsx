import {useEffect,useMemo,useState} from 'react';
import {Area,AreaChart,CartesianGrid,ResponsiveContainer,Tooltip,XAxis,YAxis} from 'recharts';
import {Activity,ArrowDownRight,ArrowUpRight,Globe2,TriangleAlert} from 'lucide-react';
import GeoMap from './GeoMap';

type Interest={brandCode:string;name:string;observedOn:string;score:number;change7d:number|null;change30d:number|null};
type Window='week'|'month'|'year';
const brands=[{code:'aeza',name:'Aéza'},{code:'hetzner',name:'Hetzner'},{code:'contabo',name:'Contabo'},{code:'timeweb_cloud',name:'Timeweb Cloud'},{code:'selectel',name:'Selectel'},{code:'firstvds',name:'FirstVDS'}];
const windows:Record<Window,string>={week:'Недели',month:'Месяцы',year:'Годы'};

async function getJson<T>(url:string):Promise<T>{const response=await fetch(url);if(!response.ok)throw new Error(`Market API: ${response.status} (${url})`);return response.json();}
function aggregate(rows:Interest[],window:Window){
 const grouped=new Map<string,{sum:number;count:number}>();
 for(const row of rows){const key=window==='week'?row.observedOn:window==='month'?row.observedOn.slice(0,7):row.observedOn.slice(0,4);const entry=grouped.get(key)??{sum:0,count:0};entry.sum+=row.score;entry.count++;grouped.set(key,entry);}
 return [...grouped].sort(([a],[b])=>a.localeCompare(b)).map(([date,value])=>({date,score:Number((value.sum/value.count).toFixed(1))}));
}
function Change({value}:{value:number|null}){if(value===null)return <span className="market-no-value">—</span>;return <span className={value>=0?'market-positive':'market-negative'}>{value>=0?<ArrowUpRight size={15}/>:<ArrowDownRight size={15}/>} {Math.abs(value).toFixed(1)} п.</span>;}

export default function Market(){
 const [interest,setInterest]=useState<Interest[]>([]);
 const [brand,setBrand]=useState('aeza'),[window,setWindow]=useState<Window>('week'),[loading,setLoading]=useState(true),[errors,setErrors]=useState<string[]>([]);
 useEffect(()=>{let active=true;getJson<Interest[]>('/api/market/search-interest').then(rows=>{if(active)setInterest(rows);}).catch(error=>{if(active)setErrors(['Поисковый интерес: '+String(error)]);}).finally(()=>{if(active)setLoading(false);});return()=>{active=false;};},[]);
 const selected=brands.find(item=>item.code===brand)!;
 const rows=useMemo(()=>interest.filter(row=>row.brandCode===brand).sort((a,b)=>a.observedOn.localeCompare(b.observedOn)),[interest,brand]);
 const latest=rows.at(-1);
 const chart=useMemo(()=>aggregate(rows,window),[rows,window]);
 const latestByBrand=brands.map(item=>({name:item.name,code:item.code,row:interest.filter(point=>point.brandCode===item.code).at(-1)}));
 const observedDates=rows.length?`${rows[0].observedOn} — ${rows.at(-1)!.observedOn}`:'Нет наблюдений';
 return <div className="market-page">
 <div className="page-heading"><div><div className="market-eyebrow">MARKET INTELLIGENCE / SEARCH DEMAND</div><h1>Брендовый интерес</h1><p>Как меняется поисковый спрос на Aéza и других VPS-провайдеров</p></div><div className="market-source-pill"><span/> GOOGLE SEARCH · TRENDSMCP</div></div>
 {loading?<div className="market-state">Загружаем рыночные данные…</div>:<>
 {errors.length>0&&<div className="market-state" role="alert"><TriangleAlert size={20}/><div><strong>Поисковые данные не загрузились</strong>{errors.map(item=><div key={item}>{item}</div>)}<small>Проверьте запущенный market_api и его сообщения в терминале.</small></div></div>}
 {!interest.length&&errors.length===0&&<div className="market-empty-overview"><div><strong>История поиска ещё не загружена</strong><p>После первого успешного сбора TrendsMCP здесь появятся недельные наблюдения по шести провайдерам.</p></div></div>}
 <div className="market-notice">Индекс 0–100 показывает интерес к бренду относительно его собственного пика. Это не количество запросов.</div>
 <div className="market-brand-grid">{latestByBrand.map(item=><button type="button" key={item.code} className={`market-brand ${brand===item.code?'selected':''}`} onClick={()=>setBrand(item.code)} aria-pressed={brand===item.code}><span>{item.name}</span><strong>{item.row?item.row.score.toFixed(0):'—'}</strong><small>{item.row?`Последняя точка ${item.row.observedOn}`:'Нет данных'}</small></button>)}</div>
 <section className="panel market-section"><div className="market-section-head"><div><div className="market-section-icon"><Activity size={19}/></div><h2>Динамика интереса · {selected.name}</h2><p>{observedDates}</p></div><div className="market-segments" role="group" aria-label="Группировка графика">{(Object.keys(windows) as Window[]).map(item=><button type="button" key={item} className={window===item?'active':''} onClick={()=>setWindow(item)} aria-pressed={window===item}>{windows[item]}</button>)}</div></div>
 {chart.length>1?<div className="market-chart"><ResponsiveContainer width="100%" height="100%"><AreaChart data={chart} margin={{top:15,right:16,left:-18,bottom:0}}><defs><linearGradient id="interestFill" x1="0" x2="0" y1="0" y2="1"><stop offset="0%" stopColor="#83d6ff" stopOpacity={0.37}/><stop offset="100%" stopColor="#83d6ff" stopOpacity={0}/></linearGradient></defs><CartesianGrid stroke="#a9dfff1b" vertical={false}/><XAxis dataKey="date" tick={{fill:'#9bb7c9',fontSize:11}} axisLine={false} tickLine={false}/><YAxis domain={[0,100]} tick={{fill:'#9bb7c9',fontSize:11}} axisLine={false} tickLine={false}/><Tooltip contentStyle={{background:'#142d42',border:'1px solid #8dd2fc55',borderRadius:10,color:'#e9f7ff'}} formatter={(value)=>[`${value} / 100`,'Интерес']}/><Area type="monotone" dataKey="score" stroke="#9edcff" strokeWidth={3} fill="url(#interestFill)" activeDot={{r:6,fill:'#dff5ff'}}/></AreaChart></ResponsiveContainer></div>:<div className="market-chart-empty"><Activity size={30}/><strong>{rows.length?'Недостаточно точек для графика':'История ещё не загружена'}</strong><span>{rows.length?'Для выбранной группировки требуется минимум две точки.':'После загрузки TrendsMCP здесь появятся недельные точки.'}</span></div>}
 <div className="market-change-row"><div><span>Текущий индекс</span><strong>{latest?.score.toFixed(0)??'—'}<small>{latest?'/ 100':''}</small></strong></div><div><span>За 7 дней</span><strong><Change value={latest?.change7d??null}/></strong></div><div><span>За 30 дней</span><strong><Change value={latest?.change30d??null}/></strong></div></div></section>
 <section className="panel market-section"><div className="market-section-head"><div><div className="market-section-icon"><Globe2 size={19}/></div><h2>География интереса · {selected.name}</h2><p>Где бренд ищут активнее в выбранном периоде</p></div></div><GeoMap brand={brand}/></section>
 </>}
 </div>;
}
