import {useEffect,useState} from 'react';
import {Link,useSearchParams} from 'react-router-dom';
import {ArrowUpRight,Info,Server} from 'lucide-react';
import {useAnalyzer} from '../hooks/useAnalyzer';
import {money,date} from '../lib/utils';
import {Badge,Panel,Empty,colors} from '../components/common';
import {GlassSelect} from '../components/ui/GlassSelect';

type RustClientReport={accountId:number;values:(number|null)[];robustZ:number[];mahalanobis:number|null;neighbours:number[];neighbourDistance:number|null;peers:number[];conditionalZ:number[]};
type RustCenter={feature:string;median:number;scale:number};
type LedgerEntry={eventId:string;accountId:number;serviceId:number|null;tariffProductId:number|null;occurredAt:string;eventType:string;amountCents:number;status:string;source:string};
type RustReport={features:string[];centers:RustCenter[];clients:RustClientReport[];correlation:(number|null)[][]};
const featureLabels:Record<string,string>={noAutoProlongShare:'Без автопродления',prolongWaitShare:'В prolong_wait',failedProlongShare:'Неуспешные продления',expiringRevenueShare:'Истекающая выручка',paymentCoverage:'Покрытие платежей'};
const metric=(feature:string,value:number|null|undefined)=>value==null?'нет данных':feature==='paymentCoverage'?`${value.toFixed(2)}×`:`${(value*100).toFixed(1)}%`;
const deviation=(value:number|undefined)=>value==null?'—':`${value>0?'+':''}${value.toFixed(2)} MAD`;

export default function Snapshot(){
 const {data,settings,risks,riskError}=useAnalyzer();
 const [params,setParams]=useSearchParams();
 const [rustReport,setRustReport]=useState<RustReport|null>(null);
 const [rustError,setRustError]=useState('');
 const [ledger,setLedger]=useState<LedgerEntry[]>([]);
 const [ledgerError,setLedgerError]=useState('');
 const selectedAccountId=data?.accounts.find(a=>a.id===params.get('client'))?.id??data?.accounts[0]?.id;
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
 if(!data||!settings)return null;
 const account=data.accounts.find(a=>a.id===params.get('client'))??data.accounts[0];
 if(!account)return <Empty title="Нет клиентов" icon={<Server/>}>Добавьте аккаунты через API для анализа.</Empty>;
 const report=rustReport?.clients.find(item=>String(item.accountId)===account.id);
 const risk=risks?.[account.id];
 const services=data.services.filter(s=>s.accountId===account.id);
 const transactions=data.transactions.filter(t=>t.accountId===account.id).sort((a,b)=>b.createdAt.localeCompare(a.createdAt));
 if(!risk)return <div className="market-state" role={riskError?'alert':'status'}>{riskError?`Snapshot Model недоступна: ${riskError}`:'Рассчитываем Snapshot Model на сервере…'}</div>;
 return <>
  <div className="page-heading"><div><div className="eyebrow">ОБЪЯСНИМАЯ АНАЛИТИКА</div><h1>Snapshot Model</h1><p>Текущее состояние клиента и причины риска.</p></div><label className="client-select">Клиент<GlassSelect value={account.id} onChange={value=>setParams({client:value})} options={data.accounts.map(item=>({value:item.id,label:`${item.name} · ${item.id}`}))}/></label></div>
  <div className="snapshot-grid">
   <Panel title={account.name} subtitle={`${account.id} · ${account.email}`}>
    <div className="big-score" style={{color:colors[risk.level]}}>{risk.score}<span>/ 100</span></div><Badge level={risk.level}/>
    <p className="body-note">{risk.factors.filter(f=>f.points>5).map(f=>f.label).join(' · ')||'Выраженные факторы риска отсутствуют.'}</p>
    <Link className="text-link" to={`/historical?client=${account.id}`}>История клиента <ArrowUpRight size={14}/></Link>
   </Panel>
   <Panel title="Баланс и ближайшие платежи" subtitle={`Окно анализа: ${settings.renewalWindowDays} дней`}>
    <div className="balance-metrics"><div><small>Доступный баланс</small><strong>{money(risk.spendableBalanceCents)}</strong></div><div><small>К оплате в окне</small><strong>{money(risk.upcomingCents)}</strong></div><div><small>Покрытие платежей</small><strong>{risk.paymentCoverage.toFixed(2)}×</strong></div></div>
    <div className="chart-note"><Info size={14}/> Значение 1× означает, что доступного баланса хватает на ближайшие платежи.</div>
   </Panel>
  </div>
  <Panel title="Snapshot Risk Score" subtitle="Доли и покрытие рассчитаны для выбранного клиента из PostgreSQL">
   <div className="factor-list">{risk.factors.map(f=><div className="factor" key={f.key}>
    <div><strong>{f.label}</strong><small>{f.available?metric(f.key,f.value):'Для признака пока нет наблюдений'} · {f.explanation}</small></div>
    <div className="factor-track"><span style={{width:`${f.severity*100}%`}}/></div><b>+{f.points.toFixed(1)}</b>
   </div>)}</div>
   <details className="formula"><summary>Как читается индекс</summary><p>Для долей риск равен самой доле. Для покрытия риск растёт, когда баланс покрывает меньше одного ближайшего платежа. Итог — взвешенная сумма пяти признаков от 0 до 100.</p></details>
  </Panel>
  <Panel title="Сравнение с другими клиентами" subtitle="Робастное отклонение по каждому признаку и отклонение внутри похожей группы">
   {report?<>
    <p className="body-note">Махаланобис: {report.mahalanobis?.toFixed(2)??'недостаточно наблюдений'} · среднее расстояние до ближайших: {report.neighbourDistance?.toFixed(2)??'—'}.</p>
    <div className="table-scroll"><table><thead><tr><th>Признак</th><th>Значение</th><th>Медиана</th><th>Отклонение MAD</th><th>Отклонение от похожих</th></tr></thead><tbody>
     {rustReport!.features.map((feature,index)=>{const center=rustReport!.centers.find(item=>item.feature===feature);return <tr key={feature}><td>{featureLabels[feature]??feature}</td><td>{metric(feature,report.values[index])}</td><td>{metric(feature,center?.median)}</td><td>{deviation(report.robustZ[index])}</td><td>{deviation(report.conditionalZ[index])}</td></tr>;})}
    </tbody></table></div>
    <p className="body-note">Ближайшие по риск-вектору: {report.neighbours.join(', ')||'нет других клиентов'}. Сопоставимые по размеру и расходам: {report.peers.join(', ')||'нет других клиентов'}.</p>
   </>:<p className="body-note">{rustError?`Сравнение пока недоступно: ${rustError}`:'Сравнение загружается.'}</p>}
  </Panel>
  {rustReport&&<Panel title="Корреляции признаков" subtitle="Pearson r; связь признаков не доказывает причинность">
   <div className="table-scroll"><table><thead><tr><th>Признак</th>{rustReport.features.map(feature=><th key={feature}>{featureLabels[feature]??feature}</th>)}</tr></thead><tbody>{rustReport.correlation.map((row,index)=><tr key={rustReport!.features[index]}><td>{featureLabels[rustReport!.features[index]]??rustReport!.features[index]}</td>{row.map((value,column)=><td key={column}>{value==null?'—':value.toFixed(2)}</td>)}</tr>)}</tbody></table></div>
  </Panel>}
  <Panel title="Услуги клиента" subtitle="Текущее состояние из PostgreSQL"><div className="table-scroll"><table><thead><tr><th>Услуга</th><th>Локация</th><th>В месяц</th><th>Продление</th><th>Статус</th><th>Автопродление</th></tr></thead><tbody>{services.map(service=><tr key={service.id}><td><strong>{service.name}</strong><small className="block muted">{service.tariffProductId?`Тариф ${service.tariffPlanKey??service.name} · ID ${service.tariffProductId}`:`Услуга ${service.id} · тариф не сопоставлен`}</small></td><td>{service.region??'—'}</td><td>{money(service.priceCents)}</td><td>{date(service.renewsAt)}</td><td><span className={`service-status ${service.status}`}>{ {active:'Активна',blocked:'Заблокирована',prolong_wait:'Ожидает продления',suspended:'Приостановлена',cancelled:'Удалена'}[service.status]}</span></td><td>{service.autoRenew?'Включено':'Выключено'}</td></tr>)}</tbody></table></div></Panel>
  <Panel title="Финансовый журнал" subtitle="Проведённые оплаты услуг и зафиксированные пополнения"><div className="table-scroll"><table><thead><tr><th>Дата</th><th>Событие</th><th>Услуга / тариф</th><th>Сумма</th><th>Источник</th></tr></thead><tbody>{ledger.map(entry=><tr key={`${entry.source}:${entry.eventId}`}><td>{date(entry.occurredAt)}</td><td>{{service_order:'Заказ услуги',service_renewal:'Продление',topup:'Пополнение',replenishment_adjustment:'Коррекция пополнений'}[entry.eventType]??entry.eventType}</td><td>{entry.serviceId??'—'}{entry.tariffProductId?` · тариф ${entry.tariffProductId}`:''}</td><td className={entry.amountCents>0?'green-text':''}>{entry.amountCents>0?'+':'−'}{money(Math.abs(entry.amountCents))}</td><td>{entry.source}</td></tr>)}</tbody></table></div>{ledgerError&&<p role="alert">Журнал недоступен: {ledgerError}</p>}{!ledgerError&&ledger.length===0&&<p className="body-note">Подтверждённых операций пока нет.</p>}<p className="body-note">Возврат отображается только при явной записи в источнике биллинга; текущая схема не содержит отдельного типа возврата.</p></Panel>
  <Panel title="Транзакции" subtitle="Операции из PostgreSQL"><div className="table-scroll"><table><thead><tr><th>ID / услуга</th><th>Дата</th><th>Тип операции</th><th>Сумма</th><th>Статус</th></tr></thead><tbody>{transactions.map(transaction=><tr key={transaction.id}><td>{transaction.id}<small className="block muted">{transaction.serviceId??'Баланс аккаунта'}</small></td><td>{date(transaction.createdAt)}</td><td>{transaction.type==='topup'?'Пополнение':'Оплата услуги'}</td><td className={transaction.type==='topup'?'green-text':''}>{transaction.type==='topup'?'+':'−'}{money(transaction.amountCents)}</td><td><span className={`badge ${transaction.status==='completed'?'low':'high'}`}>{transaction.status==='completed'?'Выполнена':'Не выполнена'}</span></td></tr>)}</tbody></table></div></Panel>
 </>;
}
