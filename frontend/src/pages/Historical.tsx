import {useEffect,useMemo,useState} from 'react';
import {Link,useSearchParams} from 'react-router-dom';
import {CartesianGrid,Line,LineChart,ResponsiveContainer,Tooltip,XAxis,YAxis} from 'recharts';
import {ArrowUpRight,History} from 'lucide-react';
import {useAnalyzer} from '../hooks/useAnalyzer';
import {Panel,Empty} from '../components/common';
import {Button} from '../components/ui/button';

type Assessment={familiarity:number;is_confident:boolean;is_unusual:boolean;anomaly_score:number|null};
type HistoryPoint={observedOn:string;assessment:Assessment;features:Record<string,number>};

export default function Historical(){
 const {data,settings}=useAnalyzer();
 const [params,setParams]=useSearchParams();
 const [points,setPoints]=useState<HistoryPoint[]>([]);
 const [loading,setLoading]=useState(false);
 const [error,setError]=useState('');
 const account=data?.accounts.find(a=>a.id===params.get('client'))??data?.accounts[0];
 useEffect(()=>{
  if(!account){setPoints([]);return;}
  const controller=new AbortController();
  setLoading(true);setError('');
  fetch(`/api/client/historical/${account.id}`,{signal:controller.signal})
   .then(async response=>{if(!response.ok)throw new Error(`API: ${response.status}`);return response.json() as Promise<HistoryPoint[]>;})
   .then(rows=>setPoints(rows.sort((a,b)=>a.observedOn.localeCompare(b.observedOn))))
   .catch(reason=>{if(!controller.signal.aborted)setError(String(reason));})
   .finally(()=>{if(!controller.signal.aborted)setLoading(false);});
  return ()=>controller.abort();
 },[account?.id]);
 const chart=useMemo(()=>points.map(point=>({date:point.observedOn,anomaly:point.assessment.anomaly_score??0})),[points]);
 if(!data||!settings)return null;
 const latest=points.at(-1);
 return <>
  <div className="page-heading"><div><div className="eyebrow">ПОВЕДЕНЧЕСКАЯ АНАЛИТИКА</div><h1>Historical Model</h1><p>Ежедневные снимки и отклонения от привычного поведения клиента.</p></div><label className="client-select">Клиент<select value={account?.id??''} onChange={event=>setParams({client:event.target.value})}>{data.accounts.map(item=><option key={item.id} value={item.id}>{item.name} · {item.id}</option>)}</select></label></div>
  <Panel title="Исторический анализ" subtitle={account?.email??'Нет аккаунтов'} action={<span className={`badge ${latest?.assessment.is_confident?'low':'medium'}`}>{latest?.assessment.is_confident?'Профиль накоплен':'Накопление истории'}</span>}>
   {loading?<p>Загружаем историю…</p>:error?<div className="error-box" role="alert">История не загрузилась: {error}</div>:points.length===0?<Empty icon={<History size={30}/>} title="Снимков пока нет">Запустите ежедневный сбор <code>cargo run --bin client_snapshot</code>. После первого запуска здесь появятся реальные наблюдения из PostgreSQL.</Empty>:<>
    <div className="history-progress"><span>Накоплено снимков</span><strong>{points.length} / {settings.historicalMinDays} дней</strong><div style={{width:`${Math.min(100,points.length/settings.historicalMinDays*100)}%`}}/></div>
    <p className="body-note">Последний снимок: {latest?.observedOn}. Знакомство модели с историей: {Math.round((latest?.assessment.familiarity??0)*100)}%. {latest?.assessment.is_unusual?'Есть отклонение от личной истории.':'Выраженного отклонения нет.'} Это не вероятность оттока.</p>
    <div style={{height:260,width:'100%'}}><ResponsiveContainer><LineChart data={chart}><CartesianGrid stroke="#a9dfff1b" vertical={false}/><XAxis dataKey="date" tick={{fill:'#9bb7c9',fontSize:11}}/><YAxis tick={{fill:'#9bb7c9',fontSize:11}}/><Tooltip contentStyle={{background:'#142d42',color:'#e9f7ff'}}/><Line type="monotone" dataKey="anomaly" name="Отклонение" stroke="#9edcff" strokeWidth={2} dot={{r:3}}/></LineChart></ResponsiveContainer></div>
   </>}
   {account&&<div className="empty-action"><Button asChild variant="default"><Link to={`/snapshot?client=${account.id}`}>Текущий анализ <ArrowUpRight size={15}/></Link></Button></div>}
  </Panel>
 </>;
}
