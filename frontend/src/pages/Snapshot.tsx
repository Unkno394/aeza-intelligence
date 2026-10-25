import {useEffect,useMemo,useRef,useState,type CSSProperties} from 'react';
import {Link,useSearchParams} from 'react-router-dom';
import {ArrowDown,ArrowUp,ArrowUpDown,ArrowUpRight,CalendarClock,CheckCircle2,ChevronDown,ChevronLeft,ChevronRight,CircleAlert,CreditCard,Info,Search,Server,Wallet} from 'lucide-react';
import {useAnalyzer} from '../hooks/useAnalyzer';
import {money,date} from '../lib/utils';
import {Badge,Empty,Panel,colors} from '../components/common';
import {AccountPicker} from '../components/ui/AccountPicker';
import {GlassSelect} from '../components/ui/GlassSelect';
import type {RiskResult,Service} from '../types';
import {fetchPriceImpact,needsPriceReview,priceIncreasePercent,priceOutcome,type PriceImpactEvent} from '../features/priceImpact';
import {demoEvidenceReady,demoPriceRisk} from '../features/demoPriceRisk';
import './snapshot.css';

type RustClientReport={accountId:number;values:(number|null)[];robustZ:number[];mahalanobis:number|null;neighbours:number[];neighbourDistance:number|null;peers:number[];conditionalZ:number[]};
type RustCenter={feature:string;median:number;scale:number};
type LedgerEntry={eventId:string;accountId:number;serviceId:number|null;tariffProductId:number|null;occurredAt:string;eventType:string;amountCents:number;status:string;source:string};
type Cashflow={startingBalanceCents:number;includedServices:number;unresolvedServices:number;windows:{days:number;scheduledRenewals:number;projectedOutflowCents:number;balanceAfterCents:number;deficitCents:number}[];assumption:string};
type RustReport={features:string[];centers:RustCenter[];clients:RustClientReport[];correlation:(number|null)[][]};
type ServiceStatusFilter='all'|Service['status'];
type ServiceSort='priceCents'|'renewsAt';
const pageSize=10;
const featureLabels:Record<string,string>={noAutoProlongShare:'Услуги без автопродления',prolongWaitShare:'Услуги в prolong_wait',failedProlongShare:'Неуспешные продления',expiringRevenueShare:'Скоро истекающая выручка',paymentCoverage:'Покрытие платежей'};
const statusLabels:Record<Service['status'],string>={active:'Активна',blocked:'Заблокирована',prolong_wait:'Ожидает оплаты',suspended:'Приостановлена',cancelled:'Удалена'};
const metric=(feature:string,value:number|null|undefined)=>value==null?'нет данных':feature==='paymentCoverage'?`${value.toFixed(2)}×`:`${(value*100).toFixed(1)}%`;
const deviation=(value:number|undefined)=>value==null?'—':`${value>0?'+':''}${value.toFixed(2)} MAD`;
const median=(values:number[])=>{const sorted=[...values].sort((a,b)=>a-b);const middle=Math.floor(sorted.length/2);return sorted.length===0?null:sorted.length%2?sorted[middle]:(sorted[middle-1]+sorted[middle])/2;};
const dateTime=(value:string)=>new Intl.DateTimeFormat('ru-RU',{dateStyle:'medium',timeStyle:'short',timeZone:'Europe/Moscow'}).format(new Date(value));
const serviceFilterText=(service:Service)=>`${service.id} ${service.name} ${service.tariffPlanKey??''} ${service.region??''}`.toLocaleLowerCase();

export default function Snapshot(){
 const {data,settings,risks,error:riskError}=useAnalyzer();
 const [params,setParams]=useSearchParams();
 const syntheticMode=params.get('mode')==='synthetic';
 const [rustReport,setRustReport]=useState<RustReport|null>(null);
 const [rustError,setRustError]=useState('');
 const [ledger,setLedger]=useState<LedgerEntry[]>([]);
 const [ledgerError,setLedgerError]=useState('');
 const [cashflow,setCashflow]=useState<Cashflow|null>(null);
 const [cashflowError,setCashflowError]=useState('');
 const [priceEvents,setPriceEvents]=useState<PriceImpactEvent[]>([]);
 const [priceImpactError,setPriceImpactError]=useState('');
 const [priceImpactLoading,setPriceImpactLoading]=useState(true);
 const [serviceSearch,setServiceSearch]=useState('');
 const [statusFilter,setStatusFilter]=useState<ServiceStatusFilter>('all');
 const [autoFilter,setAutoFilter]=useState<'all'|'enabled'|'disabled'>('all');
 const [serviceSort,setServiceSort]=useState<ServiceSort>('renewsAt');
 const [sortDirection,setSortDirection]=useState(1);
 const [servicePage,setServicePage]=useState(0);
 const [transactionFilter,setTransactionFilter]=useState<'all'|'prolongOutcomes'>('all');
 const transactionSectionRef=useRef<HTMLDivElement>(null);
 const requestedAccount=data?.accounts.find(a=>a.id===params.get('client'));
 const realRiskCandidates=data?.accounts.filter(a=>Number(a.id)>0&&!a.email.endsWith('@example.test'))??[];
 const demoRiskCandidates=data?.accounts.filter(a=>a.email.startsWith('demo-stream-'))??[];
 const riskCandidates=realRiskCandidates.length?realRiskCandidates:demoRiskCandidates;
 const preferredRiskAccount=[...riskCandidates].sort((a,b)=>(risks?.[b.id]?.score??-1)-(risks?.[a.id]?.score??-1))[0];
 const selectedAccountId=requestedAccount?.id??preferredRiskAccount?.id??data?.accounts[0]?.id;

 useEffect(()=>{
  const controller=new AbortController();
  fetch('/api/client/analysis',{signal:controller.signal})
   .then(async response=>{if(!response.ok)throw new Error(`API: ${response.status}`);return response.json() as Promise<RustReport>;})
   .then(setRustReport)
   .catch(error=>{if(!controller.signal.aborted)setRustError(String(error));});
  return ()=>controller.abort();
 },[]);

 useEffect(()=>{
  if(!selectedAccountId)return;
  const controller=new AbortController();
  setLedger([]);
  fetch(`/api/client/financial-ledger/${selectedAccountId}`,{signal:controller.signal})
   .then(async response=>{if(!response.ok)throw new Error(`API: ${response.status}`);return response.json() as Promise<LedgerEntry[]>;})
   .then(rows=>{setLedger(rows);setLedgerError('');})
   .catch(error=>{if(!controller.signal.aborted)setLedgerError(String(error));});
  return ()=>controller.abort();
 },[selectedAccountId]);

 useEffect(()=>{
  if(!selectedAccountId)return;
  const controller=new AbortController();
  setCashflow(null);
  fetch(`/api/client/cashflow/${selectedAccountId}`,{signal:controller.signal})
   .then(async response=>{if(!response.ok)throw new Error(`API: ${response.status}`);return response.json() as Promise<Cashflow>;})
   .then(value=>{setCashflow(value);setCashflowError('');})
   .catch(error=>{if(!controller.signal.aborted)setCashflowError(String(error));});
  return ()=>controller.abort();
 },[selectedAccountId]);

 useEffect(()=>{setServicePage(0);setServiceSearch('');setStatusFilter('all');setAutoFilter('all');setTransactionFilter('all');},[selectedAccountId]);

 useEffect(()=>{
  const controller=new AbortController();
  setPriceImpactError('');
  setPriceImpactLoading(true);
  setPriceEvents([]);
  const refresh=()=>fetchPriceImpact(controller.signal,syntheticMode?'synthetic':'real').then(rows=>{setPriceEvents(rows);setPriceImpactError('');}).catch(reason=>{if(!controller.signal.aborted)setPriceImpactError(String(reason));}).finally(()=>{if(!controller.signal.aborted)setPriceImpactLoading(false);});
  void refresh();
  const timer=syntheticMode?window.setInterval(()=>{void refresh();},15000):undefined;
  return()=>{controller.abort();if(timer)window.clearInterval(timer);};
 },[data?.asOf,syntheticMode]);

 const scenarioReady=useMemo(()=>syntheticMode&&demoEvidenceReady(priceEvents),[syntheticMode,priceEvents]);

 if(!data||!settings)return null;
 const account=data.accounts.find(a=>a.id===selectedAccountId)??data.accounts[0];
 if(!account)return <Empty title="Нет клиентов" icon={<Server/>}>Добавьте аккаунты через API для анализа.</Empty>;
 const baseRisk=risks?.[account.id];
 const services=data.services.filter(service=>service.accountId===account.id);
 const transactions=data.transactions.filter(transaction=>transaction.accountId===account.id).sort((a,b)=>b.createdAt.localeCompare(a.createdAt));
 const visibleTransactions=transactionFilter==='prolongOutcomes'?transactions.filter(transaction=>transaction.billingType==='prolong'):transactions;
 if(!baseRisk)return <div className="market-state" role={riskError?'alert':'status'}>{riskError?`Snapshot Model недоступна: ${riskError}`:'Рассчитываем Snapshot Model на сервере…'}</div>;
 const risk=syntheticMode?demoPriceRisk(baseRisk,settings,priceEvents.find(event=>event.accountId===account.id),scenarioReady):baseRisk;

 const report=rustReport?.clients.find(item=>String(item.accountId)===account.id);
 const peerClients=report&&rustReport?report.peers.map(id=>rustReport.clients.find(client=>client.accountId===id)).filter((client):client is RustClientReport=>Boolean(client)):[];
 const peerComparisons=report&&rustReport?rustReport.features.map((feature,index)=>{
  const center=rustReport.centers.find(item=>item.feature===feature);
  const observed=peerClients.map(client=>client.values[index]).filter((value):value is number=>value!==null&&value!==undefined);
  const imputed=peerClients.map(client=>client.values[index]??center?.median).filter((value):value is number=>value!==null&&value!==undefined);
  return {feature,value:report.values[index],peerMedian:median(imputed),observedPeers:observed.length,peerCount:peerClients.length,conditionalZ:report.conditionalZ[index]};
 }):[];
 const unusualPeerComparisons=[...peerComparisons].filter(item=>item.value!==null&&item.peerMedian!==null).sort((a,b)=>Math.abs(b.conditionalZ)-Math.abs(a.conditionalZ)).slice(0,3);
 const asOf=Date.parse(data.asOf);
 const windowEnd=asOf+settings.renewalWindowDays*86_400_000;
 const currentServices=services.filter(service=>service.status!=='cancelled');
 const currentServiceIds=new Set(currentServices.map(service=>service.id));
 const noAutoServices=currentServices.filter(service=>!service.autoRenew);
 // The dataset endpoint already converts priceCents to a monthly amount in SQL.
 const noAutoMonthlyCents=noAutoServices.reduce((sum,service)=>sum+service.priceCents,0);
 const noAutoServiceShare=currentServices.length?noAutoServices.length/currentServices.length:null;
 const noAutoRevenueShare=risk.monthlyCents>0?noAutoMonthlyCents/risk.monthlyCents:null;
 const waitingServices=currentServices.filter(service=>service.status==='prolong_wait');
 const waitingMonthlyCents=waitingServices.reduce((sum,service)=>sum+service.priceCents,0);
 const soonExpiringServices=currentServices.filter(service=>{const renews=Date.parse(service.renewsAt);return Number.isFinite(renews)&&renews>asOf&&renews<=windowEnd;});
 const attempts=transactions.filter(transaction=>transaction.billingType==='prolong');
 const unperformedAttempts=attempts.filter(transaction=>transaction.status==='unknown');
 const expiringFactor=risk.factors.find(factor=>['expiringRevenue','expiringRevenueShare'].includes(String(factor.key)));
 const expiringMonthlyCents=expiringFactor?.value==null?null:Math.round(risk.monthlyCents*expiringFactor.value);
 const availableFactorCount=risk.factors.filter(factor=>factor.available).length;
 const topFactors=[...risk.factors].filter(factor=>factor.available&&factor.points>0).sort((a,b)=>b.points-a.points).slice(0,3);
 const shortfallCents=Math.max(0,risk.upcomingCents-risk.spendableBalanceCents);
 const clientPriceEvents=priceEvents.filter(event=>String(event.accountId)===account.id);
 const recentPriceEvents=clientPriceEvents.filter(event=>Date.parse(event.changedAt)>=Date.now()-90*86_400_000);
 const priceReviewCount=recentPriceEvents.filter(event=>needsPriceReview(event)).length;
 const attentionServices=new Set<string>([
  ...noAutoServices.map(service=>service.id),
  ...waitingServices.map(service=>service.id),
  ...soonExpiringServices.map(service=>service.id),
  ...unperformedAttempts.map(transaction=>transaction.serviceId).filter((id):id is string=>id!==undefined&&currentServiceIds.has(id)),
 ]);

 const factorDetails=(factor:RiskResult['factors'][number])=>{
  const key=String(factor.key);
  const percent=factor.value===null?'нет данных':`${(factor.value*100).toFixed(1)}%`;
  const monthlyScale=(part:number)=>risk.monthlyCents>0?`${money(part)} из ${money(risk.monthlyCents)} в месяц (${(part/risk.monthlyCents*100).toFixed(1)}%)`:`${money(part)} в месяц`;
  if(key==='noAutoProlongShare'||key==='noAutoProlong')return {observation:currentServices.length?`${noAutoServices.length} из ${currentServices.length} услуг · ${percent} без автопродления`:'Нет услуг для расчёта доли',businessScale:monthlyScale(noAutoMonthlyCents)};
  if(key==='prolongWaitShare'||key==='prolongWait')return {observation:currentServices.length?`${waitingServices.length} из ${currentServices.length} услуг · ${percent} ожидают оплаты`:'Нет услуг для расчёта доли',businessScale:monthlyScale(waitingMonthlyCents)};
  if(key==='failedProlongShare'||key==='failedProlong'){
   const unresolvedAmount=unperformedAttempts.reduce((sum,transaction)=>sum+transaction.amountCents,0);
   const totalAttemptAmount=attempts.reduce((sum,transaction)=>sum+transaction.amountCents,0);
   return {observation:attempts.length?`${unperformedAttempts.length} из ${attempts.length} попыток имеют неизвестный исход`:'Нет подтверждённых неуспешных продлений',businessScale:`Сумма неопределённых операций ${money(unresolvedAmount)}${totalAttemptAmount?` из ${money(totalAttemptAmount)} по продлениям`:''}; не считается отказом`};
  }
  if(key==='expiringRevenueShare'||key==='expiringRevenue')return {observation:`${soonExpiringServices.length} услуг истекают за ${settings.renewalWindowDays} дней · ${percent} месячной стоимости`,businessScale:expiringMonthlyCents===null?'Денежная оценка недоступна':monthlyScale(expiringMonthlyCents)};
  if(key==='paymentCoverage')return {observation:risk.upcomingCents>0&&risk.paymentCoverage!==null?`Покрытие ближайших платежей ${risk.paymentCoverage.toFixed(2)}×`:'В окне нет определённых платежей; покрытие не рассчитывается',businessScale:`Баланс ${money(risk.spendableBalanceCents)} · платежи ${money(risk.upcomingCents)} · дефицит ${money(shortfallCents)}`};
  if(key==='priceRisk')return {observation:clientPriceEvents.length?`Сценарное повышение ${priceIncreasePercent(clientPriceEvents[0]).toFixed(1)}% · исход: ${priceOutcome(clientPriceEvents[0])}`:'Сценарного повышения нет',businessScale:'Синтетический сценарий; реальная оценка не меняется'};
  return {observation:factor.explanation,businessScale:'Нет данных для бизнес-оценки'};
 };

 const showFailedProlongDetails=()=>{
  setTransactionFilter('prolongOutcomes');
  window.requestAnimationFrame(()=>{
   transactionSectionRef.current?.scrollIntoView({behavior:'smooth',block:'start'});
   transactionSectionRef.current?.focus({preventScroll:true});
  });
 };

 const renderFactor=(factor:RiskResult['factors'][number])=>{
  const details=factorDetails(factor);
  const isFailedProlong=String(factor.key)==='failedProlongShare'||String(factor.key)==='failedProlong';
  return <div className="snapshot-factor-row" key={factor.key}>
   <div className="snapshot-factor-heading">{isFailedProlong?<button type="button" className="snapshot-factor-jump" aria-label="Показать исходы продлений" onClick={showFailedProlongDetails}><strong>{factor.label}</strong><ArrowDown size={13}/></button>:<strong>{factor.label}</strong>}<b>+{factor.points.toFixed(1)} балла</b></div>
   <div className="snapshot-factor-details"><div><small>Наблюдение</small><span>{factor.available?details.observation:`Нет данных · ${factor.explanation}`}</span></div><div><small>Бизнес-масштаб</small><span>{factor.available?details.businessScale:'Не рассчитан'}</span></div></div>
   <div className="snapshot-factor-track"><span style={{width:`${factor.severity*100}%`}}/></div>
  </div>;
 };

 const recommendations=[
  ...(noAutoServices.length?[{tone:'warning',title:'Проверить автопродление',text:`Выключено у ${noAutoServices.length} из ${currentServices.length} услуг (${noAutoServiceShare===null?'—':`${(noAutoServiceShare*100).toFixed(1)}%`}); на них приходится ${money(noAutoMonthlyCents)} в месяц${noAutoRevenueShare===null?'':` · ${(noAutoRevenueShare*100).toFixed(1)}% расходов`}.`,count:noAutoServices.length,icon:<CircleAlert size={17}/>}]:[]),
  ...(waitingServices.length?[{tone:'warning',title:'Проверить услуги, ожидающие оплаты',text:`${waitingServices.length} услуг сейчас в prolong_wait. Проверьте пополнение и состояние продления.`,count:waitingServices.length,icon:<CalendarClock size={17}/>}]:[]),
  ...(unperformedAttempts.length?[{tone:'info',title:'Проверить неизвестные исходы продлений',text:`${unperformedAttempts.length} транзакций имеют статус not_performed; источник не сообщает, завершились ли они отказом.`,count:unperformedAttempts.length,icon:<CreditCard size={17}/>}]:[]),
  ...(soonExpiringServices.length?[{tone:'info',title:'Проверить ближайшие даты продления',text:`${soonExpiringServices.length} услуг истекают в окне ${settings.renewalWindowDays} дней${expiringMonthlyCents==null?'':`; их доля месячных расходов — ${money(expiringMonthlyCents)}`}.`,count:soonExpiringServices.length,icon:<CalendarClock size={17}/>}]:[]),
  ...(shortfallCents>0?[{tone:'warning',title:'Проверить достаточность баланса',text:`До суммы предстоящих платежей не хватает ${money(shortfallCents)}.`,count:1,icon:<Wallet size={17}/>}]:[]),
 ];

 const filteredServices=services
  .filter(service=>(statusFilter==='all'||service.status===statusFilter)
   &&(autoFilter==='all'||(autoFilter==='enabled'?service.autoRenew:!service.autoRenew))
   &&serviceFilterText(service).includes(serviceSearch.trim().toLocaleLowerCase()))
  .sort((a,b)=>{
   const comparison=serviceSort==='priceCents'?a.priceCents-b.priceCents:a.renewsAt.localeCompare(b.renewsAt);
   return comparison*sortDirection;
  });
 const pageCount=Math.max(1,Math.ceil(filteredServices.length/pageSize));
 const currentServicePage=Math.min(servicePage,pageCount-1);
 const visibleServices=filteredServices.slice(currentServicePage*pageSize,(currentServicePage+1)*pageSize);
 const sortBy=(key:ServiceSort)=>{if(serviceSort===key)setSortDirection(direction=>-direction);else{setServiceSort(key);setSortDirection(1);}};
 const sortIcon=(key:ServiceSort)=>serviceSort===key?(sortDirection===1?<ArrowUp size={13}/>:<ArrowDown size={13}/>):<ArrowUpDown size={13}/>;
 const signalForService=(service:Service)=>{
  const signals:Array<{label:string;tone:'warning'|'info'}>=[];
  if(service.status==='cancelled')return signals;
  if(!service.autoRenew)signals.push({label:'Без автопродления',tone:'warning'});
  if(service.status==='prolong_wait')signals.push({label:'Ожидает оплаты',tone:'warning'});
  if(soonExpiringServices.some(item=>item.id===service.id))signals.push({label:`Истекает ≤ ${settings.renewalWindowDays} дн.`,tone:'info'});
  if(unperformedAttempts.some(transaction=>transaction.serviceId===service.id))signals.push({label:'Есть not_performed',tone:'info'});
  return signals;
 };

 return <div className="snapshot-page">
  <div className="page-heading snapshot-heading">
   <div><div className="eyebrow">{syntheticMode?'СИНТЕТИЧЕСКИЙ СЦЕНАРИЙ':'ОБЪЯСНИМАЯ АНАЛИТИКА'}</div><h1>Snapshot Model</h1><p>{syntheticMode?'Сценарный балл на тестовой истории цен; реальные данные не изменены':'Текущий профиль клиента · состояние на дату расчёта'}</p></div>
   <AccountPicker accounts={data.accounts} value={account.id} onChange={value=>{setParams(current=>{const next=new URLSearchParams(current);next.set('client',value);return next;});setServicePage(0);}}/>
  </div>
  <div className="price-impact-mode" role="group" aria-label="Режим ценового риска"><button type="button" className={!syntheticMode?'active':''} onClick={()=>setParams(current=>{const next=new URLSearchParams(current);next.delete('mode');return next;})}>Реальный Risk Score</button><button type="button" className={syntheticMode?'active':''} onClick={()=>setParams(current=>{const next=new URLSearchParams(current);next.set('mode','synthetic');return next;})}>Сценарный Risk Score</button></div>
  {syntheticMode&&<p className="snapshot-analysis-note">Сценарий связан с существующими ID, но повышения, продления и расходы после них сгенерированы. Ценовой фактор {scenarioReady?'прошёл проверку групп и контролей':'не прошёл проверку достаточности'}; его вес в настройках: {settings.weights.priceRisk}. Реальный Risk Score остаётся без изменений.</p>}

  <div className="snapshot-top-grid">
   <Panel title={account.name} subtitle={`${account.id} · ${account.email}`} className="snapshot-overview" action={<Link className="snapshot-history-link" to={`/historical?client=${account.id}`}>История клиента <ArrowUpRight size={14}/></Link>}>
    <div className="snapshot-overview-content">
     <div className="snapshot-score-wrap">
      <div className={`snapshot-score-gauge ${risk.level}`} style={{'--score':`${risk.score}%`,'--score-color':colors[risk.level]} as CSSProperties}>
       <div><strong>{risk.score}</strong><span>из 100</span></div>
      </div>
      <Badge level={risk.level}/>
     </div>
     <div className="snapshot-overview-details">
      <div><small>Дата расчёта</small><strong>{dateTime(data.asOf)}</strong></div>
      <div><small>Окно анализа</small><strong>{settings.renewalWindowDays} дней</strong></div>
      <div><small>Признаки с данными</small><strong className={availableFactorCount===risk.factors.length?'snapshot-data-good':'snapshot-data-partial'}>{availableFactorCount} из {risk.factors.length}</strong></div>
      <div><small>Основной сигнал</small><strong>{topFactors[0]?.label??'Нет выраженных факторов'}</strong></div>
     </div>
    </div>
    <p className="snapshot-index-note"><Info size={14}/> Индекс риска по текущим данным; это не вероятность оттока.</p>
   </Panel>

   <Panel title="Финансовое состояние" subtitle={`Суммы и платежи в окне ${settings.renewalWindowDays} дней`} className="snapshot-finance">
    <div className="snapshot-finance-grid">
     <div className="snapshot-finance-item"><Wallet/><span>Доступный баланс</span><strong>{money(risk.spendableBalanceCents)}</strong></div>
     <div className="snapshot-finance-item"><CreditCard/><span>Ближайшие платежи</span><strong>{money(risk.upcomingCents)}</strong></div>
     <div className="snapshot-finance-item"><Server/><span>Покрытие платежей</span><strong>{risk.paymentCoverage===null?'—':`${risk.paymentCoverage.toFixed(2)}×`}</strong></div>
     <div className="snapshot-finance-item"><Wallet/><span>Месячные расходы</span><strong>{money(risk.monthlyCents)}</strong></div>
     <div className="snapshot-finance-item"><CalendarClock/><span>Скоро истекающая выручка</span><strong>{expiringMonthlyCents==null?'Нет данных':money(expiringMonthlyCents)}</strong></div>
     <div className="snapshot-finance-item"><Server/><span>Услуги в анализе</span><strong>{currentServices.length}</strong></div>
    </div>
    <div className={`snapshot-coverage-note ${risk.upcomingCents===0?'':shortfallCents>0?'is-warning':'is-covered'}`}>
     {risk.upcomingCents===0?<Info size={15}/>:shortfallCents>0?<CircleAlert size={15}/>:<CheckCircle2 size={15}/>}
     {risk.upcomingCents===0?'В выбранном окне платежей нет; устойчивость по этому признаку не оценивается.':shortfallCents>0?`Дефицит до ближайших платежей: ${money(shortfallCents)}.`:'Доступный баланс покрывает платежи в выбранном окне.'}
    </div>
   </Panel>
  </div>

  <Panel title="Сигнал: повышение стоимости" subtitle={syntheticMode?'Сценарное повышение и сгенерированное поведение':'Фактические изменения цены услуг этого клиента и последующие продления'} className="snapshot-price-impact" action={<Link className="snapshot-history-link" to={clientPriceEvents.length?`/price-impact?client=${account.id}&mode=${syntheticMode?'synthetic':'real'}`:`/price-impact?mode=${syntheticMode?'synthetic':'real'}`}>Вся история <ArrowUpRight size={14}/></Link>}>
   {priceImpactLoading?<p className="snapshot-analysis-note">Загружаем историю стоимости…</p>:priceImpactError?<p className="snapshot-analysis-note" role="alert">История стоимости недоступна: {priceImpactError}</p>:clientPriceEvents.length===0?<p className="snapshot-analysis-note">Изменений стоимости услуг клиента в журнале за последние 12 месяцев нет.</p>:<>
    <div className="snapshot-price-impact-summary"><div><small>Повышений за 12 месяцев</small><strong>{clientPriceEvents.length}</strong></div><div><small>За последние 90 дней</small><strong>{recentPriceEvents.length}</strong></div><div><small>На контроле</small><strong>{priceReviewCount}</strong></div></div>
    <div className="snapshot-price-impact-list">{clientPriceEvents.slice(0,3).map(event=><div key={event.eventId}><strong>{event.planName} · +{priceIncreasePercent(event).toFixed(1)}%</strong><span>{date(event.changedAt)} · продления за 30 дней до/после: {event.renewalsBefore}/{event.renewalsAfter} · {({renewed:'продлил',adverse:'негативное событие',unknown:'исход неизвестен',pending:'наблюдение продолжается',not_due:'срок продления не наступал'} as const)[priceOutcome(event)]}</span></div>)}</div>
   </>}
   <p className="snapshot-analysis-note">{syntheticMode?'В сценарном режиме ценовой фактор влияет на показанный балл только при достаточности данных и положительном весе. В реальный Risk Score он не входит.':'Этот сигнал показывает наблюдаемое поведение после изменения цены. Совпадение по времени не устанавливает причину ухода и пока не добавляет баллы к Risk Score.'}</p>
  </Panel>

  <div className="snapshot-reasons-grid">
   <Panel title="Главные причины риска" subtitle="Показаны факторы с наибольшим вкладом в Risk Score" className="snapshot-reasons">
    {topFactors.length===0?<div className="snapshot-empty-note"><CheckCircle2 size={17}/> Нет факторов с положительным вкладом в текущую оценку.</div>:
     <div className="snapshot-factor-list">{topFactors.map(renderFactor)}</div>}
    <details className="snapshot-all-factors"><summary>Все 5 факторов и метод расчёта <ChevronDown size={15}/></summary>
     <div className="snapshot-factor-list">{risk.factors.map(renderFactor)}</div>
     <p className="snapshot-method-note">Баллы показывают вклад признака в индекс, а не вероятность ухода. Настроенные веса нормализуются при расчёте: платежи {settings.weights.paymentCoverage}, без автопродления {settings.weights.noAutoProlong}, prolong_wait {settings.weights.prolongWait}, подтверждённые отказы {settings.weights.failedProlong}, истекающая выручка {settings.weights.expiringRevenue}. Неопределённый исход исключается.</p>
    </details>
    <aside className="snapshot-scale-example"><div className="snapshot-scale-example-title"><strong>Пример: клиент с 200 услугами</strong><span>Условные цифры</span></div><p>У 40 услуг отключено автопродление — это 20%. У клиента с 25 услугами и 5 отключёнными доля та же, хотя абсолютное число другое. Доля делает сравнение масштаба честнее, чем сумма проблемных услуг.</p><p>Если эти 40 услуг стоят €160 из €2&nbsp;000 в месяц, денежная экспозиция — 8%. При балансе €1&nbsp;200 и ближайших платежах €800 покрытие составляет 1,5×: число сигналов само по себе не означает дефицит денег. Стоимость услуг показана как контекст, а не отдельный вес Risk Score.</p></aside>
   </Panel>

   <Panel title="Что стоит проверить" subtitle="Подсказки сформированы из текущих фактов, без предположений о причинах ухода" className="snapshot-actions">
    {recommendations.length===0?<div className="snapshot-empty-note"><CheckCircle2 size={17}/> Сейчас нет конкретных сигналов для проверки по этим признакам.</div>:
     <div className="snapshot-action-list">{recommendations.slice(0,5).map((item,index)=><div className={`snapshot-action-card ${item.tone}`} key={`${item.title}-${index}`}>
      <span className="snapshot-action-icon">{item.icon}</span><div><strong>{item.title}</strong><p>{item.text}</p></div><span className="snapshot-action-count">{item.count}</span>
     </div>)}</div>}
    <p className="snapshot-action-footnote">Это проверочные подсказки по данным аккаунта, а не автоматически подтверждённые рекомендации.</p>
   </Panel>
  </div>

  <Panel title="Услуги клиента" subtitle={`${attentionServices.size} из ${currentServices.length} действующих услуг имеют сигналы · показано ${filteredServices.length} из ${services.length} строк`} className="snapshot-services">
   <div className="snapshot-service-tools">
    <label className="snapshot-service-search"><Search size={16}/><input value={serviceSearch} onChange={event=>{setServiceSearch(event.target.value);setServicePage(0);}} placeholder="Поиск по услуге, тарифу или региону…"/></label>
    <label>Статус<GlassSelect aria-label="Фильтр по статусу услуги" value={statusFilter} onChange={value=>{setStatusFilter(value as ServiceStatusFilter);setServicePage(0);}} options={[{value:'all',label:'Все статусы'},...Object.entries(statusLabels).map(([value,label])=>({value,label}))]}/></label>
    <label>Автопродление<GlassSelect aria-label="Фильтр по автопродлению" value={autoFilter} onChange={value=>{setAutoFilter(value as 'all'|'enabled'|'disabled');setServicePage(0);}} options={[{value:'all',label:'Любое'},{value:'enabled',label:'Включено'},{value:'disabled',label:'Выключено'}]}/></label>
   </div>
   <div className="table-scroll snapshot-service-scroll"><table className="snapshot-service-table"><thead><tr><th>Услуга / тариф</th><th>Регион</th><th><button className="snapshot-sort-button" onClick={()=>sortBy('priceCents')}>В месяц {sortIcon('priceCents')}</button></th><th><button className="snapshot-sort-button" onClick={()=>sortBy('renewsAt')}>Продление {sortIcon('renewsAt')}</button></th><th>Статус</th><th>Автопродление</th><th>Сигналы</th></tr></thead><tbody>
    {visibleServices.map(service=>{const signals=signalForService(service);return <tr key={service.id}>
     <td><strong>{service.tariffPlanKey??service.name}</strong><small>{service.tariffPlanKey?service.name:`Услуга ${service.id}`}</small></td>
     <td>{service.region??'Не указан'}</td><td className="numeric">{money(service.priceCents)}</td><td>{date(service.renewsAt)}</td>
     <td><span className={`snapshot-service-status ${service.status}`}>{statusLabels[service.status]}</span></td>
     <td><span className={`snapshot-autorenew ${service.autoRenew?'enabled':'disabled'}`}>{service.autoRenew?'Включено':'Выключено'}</span></td>
     <td>{signals.length?<div className="snapshot-signal-chips">{signals.map(signal=><span className={signal.tone} key={signal.label}>{signal.label}</span>)}</div>:<span className="snapshot-no-signal">—</span>}</td>
    </tr>;})}
   </tbody></table>{visibleServices.length===0&&<div className="snapshot-no-services">Услуги не найдены. Измените фильтры или поисковый запрос.</div>}</div>
   <div className="snapshot-service-pagination"><span>Показано {filteredServices.length?currentServicePage*pageSize+1:0}–{Math.min((currentServicePage+1)*pageSize,filteredServices.length)} из {filteredServices.length}</span><div><button aria-label="Предыдущая страница услуг" disabled={currentServicePage===0} onClick={()=>setServicePage(currentServicePage-1)}><ChevronLeft size={16}/></button><span>{currentServicePage+1} / {pageCount}</span><button aria-label="Следующая страница услуг" disabled={currentServicePage>=pageCount-1} onClick={()=>setServicePage(currentServicePage+1)}><ChevronRight size={16}/></button></div></div>
  </Panel>

  <details className="snapshot-analysis-disclosure">
   <summary><span><strong>Сравнение с другими клиентами</strong><small>Почему профиль необычен для похожей группы · отдельный анализ, не часть Risk Score</small></span><ChevronDown size={18}/></summary>
   <div className="snapshot-analysis-content">
    {report?<>
     <div className="snapshot-risk-vs-anomaly"><div><small>Текущий риск</small><strong>{risk.score}/100 · {({low:'низкий',medium:'средний',high:'высокий',limited:'ограниченные данные',insufficient:'недостаточно данных'} as const)[risk.level]}</strong><p>Вклад известных проблем в настраиваемый индекс · доступно факторов: {risk.availableFactors} из {risk.factors.length}.</p></div><div><small>Статистическая необычность</small><strong>Махаланобис {report.mahalanobis?.toFixed(2)??'—'}</strong><p>Насколько профиль необычен как сочетание признаков; показатель не направлен только в сторону риска.</p></div></div>
     <div className="snapshot-comparison-stats"><div><small>Клиентов в сравнительной базе</small><strong>{rustReport?.clients.length??0}</strong></div><div><small>Похожих клиентов · максимум 3</small><strong>{peerClients.length}</strong></div><div><small>Среднее расстояние до ближайших по профилю</small><strong>{report.neighbourDistance?.toFixed(2)??'—'}</strong></div></div>
     <p className="snapshot-analysis-note snapshot-comparison-window">Сравнение рассчитано на фиксированном горизонте 7 дней; текущий Risk Score использует окно {settings.renewalWindowDays} дней. Уверенность не калибруется: ниже показаны фактические размеры базы и группы.</p>
     <Panel title="Почему клиент выделяется среди похожих" subtitle="Группа подобрана по числу услуг, месячным расходам и доле коротких периодов оплаты">
      {unusualPeerComparisons.length?<div className="snapshot-peer-explanations">{unusualPeerComparisons.map(item=>{
       const direction=item.conditionalZ===0?'около медианы группы':item.conditionalZ>0?'выше по шкале риск-признаков':'ниже по шкале риск-признаков';
       const missing=item.peerCount-item.observedPeers;
       return <div className="snapshot-peer-card" key={item.feature}><div className="snapshot-peer-card-heading"><strong>{featureLabels[item.feature]??item.feature}</strong><b>{deviation(item.conditionalZ)}</b></div><div className="snapshot-peer-value-grid"><div><small>Этот клиент</small><strong>{metric(item.feature,item.value)}</strong></div><div><small>Медиана похожей группы</small><strong>{metric(item.feature,item.peerMedian)}</strong></div></div><p>{direction} среди {peerClients.length} похожих клиентов. Это сигнал для проверки профиля, не доказанная причина оттока.</p>{missing>0&&<small className="snapshot-peer-sample">По этому признаку фактические значения есть у {item.observedPeers} из {item.peerCount}; для сравнения пропуски заполнены медианой общей базы.</small>}</div>;
      })}</div>:<p className="snapshot-analysis-note">Недостаточно похожих клиентов или доступных признаков, чтобы объяснить отличие от сопоставимой группы.</p>}
      <p className="snapshot-analysis-note">Статистическое отличие — только сигнал, который помогает выбрать, что проверить. Оно не прибавляется к Risk Score, не доказывает причину оттока и не означает вероятность оттока.</p>
     </Panel>
     <Panel title="Сравнение признаков" subtitle="MAD показывает отклонение от общей базы; условное отклонение — от медианы похожей группы">
      <div className="table-scroll"><table><thead><tr><th>Признак</th><th>Значение</th><th>Медиана базы</th><th>Медиана похожих</th><th>Отклонение MAD</th><th>Отличие от похожих</th></tr></thead><tbody>
       {rustReport!.features.map((feature,index)=>{const center=rustReport!.centers.find(item=>item.feature===feature);const peer=peerComparisons.find(item=>item.feature===feature);return <tr key={feature}><td>{featureLabels[feature]??feature}</td><td>{metric(feature,report.values[index])}</td><td>{metric(feature,center?.median)}</td><td>{metric(feature,peer?.peerMedian)}</td><td>{deviation(report.robustZ[index])}</td><td>{deviation(report.conditionalZ[index])}</td></tr>;})}
      </tbody></table></div>
      <p className="snapshot-analysis-note">Ближайшие по риск-профилю: {report.neighbours.join(', ')||'нет других клиентов'}. Сопоставимая группа: {report.peers.join(', ')||'нет других клиентов'}. Для каждого признака указан размер группы и число фактических наблюдений; API не выдаёт отдельную вероятность или калиброванную уверенность сравнения.</p>
     </Panel>
     {rustReport&&<details className="snapshot-correlation-disclosure"><summary>Показать корреляционную матрицу признаков <ChevronDown size={15}/></summary><div className="table-scroll"><table><thead><tr><th>Признак</th>{rustReport.features.map(feature=><th key={feature}>{featureLabels[feature]??feature}</th>)}</tr></thead><tbody>{rustReport.correlation.map((row,index)=><tr key={rustReport!.features[index]}><td>{featureLabels[rustReport!.features[index]]??rustReport!.features[index]}</td>{row.map((value,column)=><td key={column}>{value==null?'—':value.toFixed(2)}</td>)}</tr>)}</tbody></table></div><p className="snapshot-analysis-note">Корреляция описывает совместное изменение признаков и сама по себе не устанавливает причинность.</p></details>}
    </>:<p className="snapshot-analysis-note">{rustError?`Сравнение недоступно: ${rustError}`:'Загружаем сравнение…'}</p>}
    <details className="snapshot-method-disclosure"><summary>Почему используются эти методы и когда они полезны <ChevronDown size={15}/></summary><div className="snapshot-method-grid">
     <article><strong>Risk Score · текущие сигналы</strong><ul><li><b>Решает:</b> сводит наблюдаемые сигналы в проверяемый балл и показывает вклад каждого.</li><li><b>Полезен:</b> для быстрой сортировки клиентов, когда размеченной истории уходов мало.</li><li><b>Не умеет:</b> выдавать вероятность оттока или доказывать, что сигнал вызовет уход.</li></ul></article>
     <article><strong>MAD · отдельные признаки</strong><ul><li><b>Решает:</b> показывает, насколько одно значение далеко от медианы базы.</li><li><b>Полезен:</b> для долей и покрытия платежей, когда в базе встречаются экстремальные значения.</li><li><b>Не умеет:</b> учитывать сочетания признаков или объяснять причину отклонения.</li></ul></article>
     <article><strong>Махаланобис · сочетание признаков</strong><ul><li><b>Решает:</b> выявляет необычное сочетание взаимосвязанных показателей.</li><li><b>Полезен:</b> когда показатели по отдельности обычны, но вместе встречаются редко.</li><li><b>Не умеет:</b> отличать рискованное отклонение от благоприятного или доказывать причинность.</li></ul></article>
     <article><strong>k-NN · похожие клиенты</strong><ul><li><b>Решает:</b> сравнивает профиль с ближайшими клиентами или группой похожего масштаба.</li><li><b>Полезен:</b> чтобы дать контекст клиентам с разным числом услуг и расходами.</li><li><b>Не умеет:</b> объяснять причины поведения; при малой базе группа может быть ненадёжной.</li></ul></article>
     <article><strong>Historical Model · личная норма</strong><ul><li><b>Решает:</b> показывает изменения клиента относительно его собственной истории.</li><li><b>Полезен:</b> когда накоплено достаточно снимков и событий для сравнения динамики.</li><li><b>Не умеет:</b> восстановить отсутствующую историю или доказать, почему произошло изменение.</li></ul><Link to={`/historical?client=${account.id}`}>Открыть Historical Model <ArrowUpRight size={13}/></Link></article>
    </div></details>
   </div>
  </details>

  <div className="snapshot-bottom-grid">
  <Panel title="Сценарий денежных обязательств" subtitle="Оценка предстоящих продлений" className="snapshot-cashflow">
    {cashflowError?<p className="snapshot-analysis-note" role="alert">Прогноз недоступен: {cashflowError}</p>:!cashflow?<p className="snapshot-analysis-note">Рассчитываем…</p>:<><div className="snapshot-cashflow-grid">{cashflow.windows.map(window=><div key={window.days}><small>{window.days} дней</small><strong>{money(window.projectedOutflowCents)}</strong><span>{window.scheduledRenewals} продлений</span><em className={window.deficitCents?'is-deficit':''}>{window.deficitCents?`Дефицит ${money(window.deficitCents)}`:`Баланс после: ${money(window.balanceAfterCents)}`}</em></div>)}</div><p className="snapshot-analysis-note">Начальный доступный баланс: {money(cashflow.startingBalanceCents)} · учтено услуг: {cashflow.includedServices}{cashflow.unresolvedServices?` · срок не определён у ${cashflow.unresolvedServices}`:''}.</p></>}
   </Panel>
   <Panel title="Финансовый журнал" subtitle="Пополнения, платежи и продления с известным источником" className="snapshot-ledger">
    {ledgerError?<p className="snapshot-analysis-note" role="alert">Журнал недоступен: {ledgerError}</p>:ledger.length===0?<p className="snapshot-analysis-note">Подтверждённых записей в журнале пока нет.</p>:<div className="table-scroll"><table><thead><tr><th>Дата</th><th>Событие</th><th>Услуга / тариф</th><th>Сумма</th><th>Статус</th><th>Источник</th></tr></thead><tbody>{ledger.map(entry=><tr key={`${entry.source}:${entry.eventId}`}>
     <td>{date(entry.occurredAt)}</td><td>{({service_order:'Заказ услуги',service_renewal:'Продление',topup:'Пополнение',replenishment_adjustment:'Коррекция пополнений'} as Record<string,string>)[entry.eventType]??entry.eventType}</td>
     <td>{entry.serviceId??'—'}{entry.tariffProductId?` · тариф ${entry.tariffProductId}`:''}</td><td className={entry.amountCents>0?'green-text':'numeric'}>{entry.amountCents>0?'+':''}{money(entry.amountCents)}</td><td>{({performed:'Проведено',recorded:'Зафиксировано'} as Record<string,string>)[entry.status]??entry.status}</td><td>{entry.source}</td>
    </tr>)}</tbody></table></div>}
    <p className="snapshot-analysis-note">Сумма пополнений и оплата услуги — разные события; возвраты видны только при наличии явной записи в источнике.</p>
   </Panel>
   <div className="snapshot-transactions-anchor" ref={transactionSectionRef} tabIndex={-1}>
    <Panel title="Транзакции" subtitle={transactionFilter==='prolongOutcomes'?'Попытки продления и их известные исходы':'Операции по услугам из PostgreSQL'} className="snapshot-transactions">
     {transactionFilter==='prolongOutcomes'&&<div className="snapshot-transaction-filter"><span>Все попытки продления · {visibleTransactions.length} из {transactions.length}</span><button type="button" onClick={()=>setTransactionFilter('all')}>Показать все</button></div>}
     {visibleTransactions.length===0?<p className="snapshot-analysis-note">{transactionFilter==='prolongOutcomes'?'У этого клиента нет попыток продления.':'Транзакций по услугам пока нет.'}</p>:<div className="table-scroll"><table><thead><tr><th>Дата</th><th>Тип</th><th>Услуга</th><th>Сумма</th><th>Статус</th></tr></thead><tbody>{visibleTransactions.map(transaction=><tr key={transaction.id}>
      <td>{date(transaction.createdAt)}</td><td>{transaction.billingType==='prolong'?'Продление':'Заказ услуги'}</td><td>{services.find(service=>service.id===transaction.serviceId)?.tariffPlanKey??transaction.serviceId??'—'}</td>
     <td className="numeric">{money(transaction.amountCents)}</td><td><span className={`snapshot-transaction-status ${transaction.status}`}>{transaction.status==='completed'?'Выполнена':transaction.status==='unknown'?'Исход неизвестен':'Ошибка подтверждена'}</span></td>
     </tr>)}</tbody></table></div>}
    </Panel>
   </div>
  </div>
 </div>;
}
