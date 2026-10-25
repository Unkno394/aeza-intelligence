import {useMemo,useRef,useState} from 'react';
import {Link} from 'react-router-dom';
import {ArrowDown,ArrowUpRight,ArrowUpDown,Users,Server,Wallet,ShieldAlert,Search,Download,SlidersHorizontal,ChevronLeft,ChevronRight,Info,ChevronDown,FileSpreadsheet,FileText,Code2,Table2} from 'lucide-react';
import {PieChart,Pie,Cell,ResponsiveContainer,BarChart,Bar,XAxis,YAxis,Tooltip,CartesianGrid} from 'recharts';
import {useAnalyzer} from '../hooks/useAnalyzer';
import {money} from '../lib/utils';
import {Badge,Panel,colors} from '../components/common';
import {Button} from '../components/ui/button';
import {GlassSelect} from '../components/ui/GlassSelect';
import {ChartTooltip} from '../components/ChartTooltip';
import type {RiskLevel} from '../types';
import {exportRiskReportCsv,exportRiskReportPdf,exportRiskReportSql,exportRiskReportXlsx} from '../lib/risk-report-export';
import {selectCustomerRows,type CustomerSortKey} from '../features/risk/customerTable';
export default function Dashboard(){
 const {data,settings,risks}=useAnalyzer();const [search,setSearch]=useState(''),[filter,setFilter]=useState<"all"|RiskLevel>('all'),[sort,setSort]=useState<CustomerSortKey>('score'),[direction,setDirection]=useState<1|-1>(-1),[page,setPage]=useState(0),[limit,setLimit]=useState(5),[pdfBusy,setPdfBusy]=useState(false),[exportError,setExportError]=useState('');const exportMenu=useRef<HTMLDetailsElement>(null);
 const rows=useMemo(()=>data&&risks?data.accounts.flatMap(a=>risks[a.id]?[{...a,...risks[a.id]}]:[]):[],[data,risks]);
 if(!data||!settings)return null;
 if(!risks)return <div className="market-state" role="status">Загружаем Risk Score из Rust API…</div>;
 if(rows.length!==data.accounts.length)return <div className="market-state" role="alert">Risk Score получен не для всех клиентов: {rows.length} из {data.accounts.length}.</div>;
 const counts=(['low','medium','high','limited','insufficient'] as RiskLevel[]).map(level=>({level,name:{low:'Низкий риск',medium:'Средний риск',high:'Высокий риск',limited:'Ограниченные данные',insufficient:'Недостаточно данных'}[level],value:rows.filter(a=>a.level===level).length}));
 // Synthetic rows (demo stream, fixtures, price scenarios) have negative IDs; the email
 // domain says nothing, the given dataset itself uses @example.test.
 const realRows=rows.filter(r=>Number(r.id)>=0);
 const fixtureCount=rows.length-realRows.length;
 const monthlyRevenueByAccount=new Map<string,number>();
 for(const service of data.services)if(service.status==='active'||service.status==='suspended')monthlyRevenueByAccount.set(service.accountId,(monthlyRevenueByAccount.get(service.accountId)??0)+service.priceCents);
 const exposure=realRows.filter(r=>r.level==='high').reduce((sum,account)=>sum+(monthlyRevenueByAccount.get(account.id)??0),0);
 const expectedLoss=realRows.reduce((sum,account)=>account.assessmentStatus==='sufficient_data'?sum+Math.round((monthlyRevenueByAccount.get(account.id)??0)*account.score/100):sum,0);
 const unassessedRevenue=realRows.filter(account=>account.assessmentStatus!=='sufficient_data').reduce((sum,account)=>sum+(monthlyRevenueByAccount.get(account.id)??0),0);
 // Upcoming payments in the renewal window that the spendable balance does not cover.
 const paymentDeficit=realRows.reduce((sum,account)=>sum+Math.max(0,account.upcomingCents-account.spendableBalanceCents),0);
 const table=selectCustomerRows(rows,{search,risk:filter,sort,direction,page,limit});const {ordered,totalPages,currentPage}=table;
 const sortBy=(key:CustomerSortKey)=>{if(sort===key)setDirection(direction===1?-1:1);else{setSort(key);setDirection(-1);}};
 const columns:[CustomerSortKey,string][]=[['id','Клиент'],['email','Email'],['balanceCents','Баланс'],['topupsCents','Пополнения'],['score','Risk Score'],['level','Уровень риска']];
 const exportReport={rows:ordered,renewalWindowDays:settings.renewalWindowDays,createdAt:new Date().toISOString()};
 const closeExportMenu=()=>{if(exportMenu.current)exportMenu.current.open=false;setExportError('');};
 const exportPdf=async()=>{closeExportMenu();setPdfBusy(true);try{await exportRiskReportPdf(exportReport);}catch(error){setExportError(error instanceof Error?error.message:'Не удалось создать PDF.');}finally{setPdfBusy(false);}};
 const buckets=[{name:'0–19',value:rows.filter(r=>r.score<20).length},{name:'20–39',value:rows.filter(r=>r.score>=20&&r.score<40).length},{name:'40–59',value:rows.filter(r=>r.score>=40&&r.score<60).length},{name:'60–79',value:rows.filter(r=>r.score>=60&&r.score<80).length},{name:'80–100',value:rows.filter(r=>r.score>=80).length}];
 return (
  <div className="dashboard-page">
   <div className="page-heading">
    <div><div className="eyebrow">ОБЗОР ПЛАТФОРМЫ</div><h1>Dashboard<span className="title-dot"/></h1><p>Понимайте риск. Сохраняйте клиентов.</p></div>
    <details className="export-menu" ref={exportMenu}>
     <summary className="button button-primary"><Download size={15}/>Экспорт отчёта<ChevronDown size={14}/></summary>
     <div className="export-menu-options">
      <button onClick={()=>{exportRiskReportCsv(exportReport);closeExportMenu();}}><Table2 size={15}/><span>CSV<small>Таблица для импорта</small></span></button>
      <button onClick={()=>{exportRiskReportSql(exportReport);closeExportMenu();}}><Code2 size={15}/><span>SQL<small>Временная таблица PostgreSQL</small></span></button>
      <button disabled={pdfBusy} onClick={()=>void exportPdf()}><FileText size={15}/><span>PDF<small>{pdfBusy?'Создаём документ…':'Отчёт для просмотра и печати'}</small></span></button>
      <button onClick={()=>{exportRiskReportXlsx(exportReport);closeExportMenu();}}><FileSpreadsheet size={15}/><span>Excel (.xlsx)<small>Книга с фильтром и сортировкой</small></span></button>
     </div>
    </details>
   </div>
   {fixtureCount>0&&<div className="dashboard-data-note" role="status">Демонстрационная база: {fixtureCount} из {rows.length} аккаунтов — тестовые. Их текущий Risk Score показан для проверки модели. Ценовой сценарий с отдельным баллом доступен в <Link to="/price-impact?mode=synthetic">разделе «Ценовой риск»</Link>; реальные финансовые метрики исключают тестовые аккаунты.</div>}
   {exportError&&<div className="export-error" role="alert">{exportError}</div>}
   <div className="metrics">{[
    {label:'Всего клиентов',value:rows.length,icon:Users,foot:`${rows.length} аккаунтов в анализе`,tone:'blue',title:undefined},
    {label:'Активные услуги',value:data.services.filter(s=>s.status==='active').length,icon:Server,foot:`Из ${data.services.length} услуг в портфеле`,tone:'purple',title:undefined},
    {label:'Клиенты высокого риска',value:counts[2].value,icon:ShieldAlert,foot:`${Math.round(counts[2].value/Math.max(rows.length,1)*100)}% от клиентской базы`,tone:'red',title:undefined},
    {label:'Ожидаемые потери · 30 дней',value:money(expectedLoss),icon:Wallet,foot:`Не покрыто балансами: ${money(paymentDeficit)} · MRR высокого риска: ${money(exposure)} · вне оценки: ${money(unassessedRevenue)}`,tone:'orange',title:'Сценарная оценка: месячная выручка активных и приостановленных услуг × Risk Score / 100. В оценку входят только клиенты с достаточными данными. Баланс аккаунта входит в Risk Score через покрытие ближайших платежей; «не покрыто балансами» — сумма ближайших платежей сверх доступного баланса. Risk Score — не калиброванная вероятность оттока; сумма показывает валовую выручку, не прибыль.'},
   ].map(m=><div className="metric" key={m.label} title={m.title}><div className="metric-top">{m.label}<span className={`metric-icon ${m.tone}`}><m.icon size={17}/></span></div><strong>{m.value}</strong><div className="metric-foot"><span className={`tiny-dot ${m.tone}`}/>{m.foot}</div></div>)}</div>
   <div className="chart-note"><Info size={13}/> 30-дневная оценка = месячная выручка активных и приостановленных услуг × Risk Score / 100; баланс аккаунта входит в Risk Score через покрытие ближайших платежей. «Не покрыто балансами» — ближайшие платежи сверх доступного баланса. В расчёт входят только клиенты с достаточными данными; это сценарий валовой выручки, не калиброванный прогноз оттока и не прибыль.</div>
   <div className="chart-grid">
    <Panel title="Распределение риска" subtitle="Текущее состояние клиентской базы" action={<span className="mini-label">{rows.length} АККАУНТОВ</span>}>
     <div className="donut-layout">
      <div className="donut"><ResponsiveContainer width="100%" height={190}><PieChart><Pie data={counts} innerRadius={65} outerRadius={82} paddingAngle={5} dataKey="value" stroke="none" startAngle={90} endAngle={-270}>{counts.map(c=><Cell key={c.level} fill={colors[c.level]}/>)}</Pie><Tooltip content={<ChartTooltip/>} offset={16} wrapperStyle={{zIndex:10,pointerEvents:'none'}}/></PieChart></ResponsiveContainer><div className="donut-center"><strong>{rows.length}</strong><span>клиентов</span></div></div>
      <div className="legend">{counts.map(c=><button key={c.level} onClick={()=>{setFilter(filter===c.level?'all':c.level);setPage(0);}}><span style={{background:colors[c.level]}}/>{c.name}<strong>{c.value}</strong><small>{Math.round(c.value/Math.max(1,rows.length)*100)}%</small></button>)}</div>
     </div>
     <div className="chart-note"><Info size={13}/> Уровни риска определяются настройками модели</div>
    </Panel>
    <Panel title="Структура клиентского портфеля" subtitle="Распределение аккаунтов по Risk Score" action={<span className="mini-label"><span className="tiny-dot blue"/> SNAPSHOT</span>}>
     <div style={{width:'100%',height:190}}><ResponsiveContainer><BarChart data={buckets} margin={{left:8,right:10,top:12,bottom:0}}><CartesianGrid strokeDasharray="3 4" vertical={false} stroke="#ffffff0d"/><XAxis dataKey="name" axisLine={false} tickLine={false} tick={{fontSize:14,fill:'#8a94a4'}}/><YAxis width={56} tickMargin={8} allowDecimals={false} axisLine={false} tickLine={false} tick={{fontSize:14,fill:'#8a94a4'}}/><Tooltip content={<ChartTooltip/>} cursor={{fill:'#ffffff06'}} wrapperStyle={{zIndex:10,pointerEvents:'none'}}/><Bar dataKey="value" name="Клиентов" fill="#d58b47" radius={[5,5,0,0]} barSize={43}>{buckets.map((_,i)=><Cell key={i} fill={['#61879b','#70a6c0','#82c2df','#a5d8ff','#d2eeff'][i]}/>)}</Bar></BarChart></ResponsiveContainer></div>
     <div className="chart-note"><span>Низкий риск</span><div className="risk-spectrum"/><span>Высокий риск</span></div>
    </Panel>
   </div>
   <Panel title="Клиенты" subtitle="Все аккаунты и ключевые показатели риска" action={<span className="count-tag">{rows.length} клиентов</span>} className="clients-panel">
    <div className="table-tools"><label className="search"><Search size={16}/><input value={search} onChange={e=>{setSearch(e.target.value);setPage(0);}} placeholder="Поиск по email или ID клиента…"/></label><label className="filter-label"><SlidersHorizontal size={15}/><GlassSelect aria-label="Фильтр риска" value={filter} onChange={value=>{setFilter(value as "all"|RiskLevel);setPage(0);}} options={[{value:'all',label:'Все уровни риска'},{value:'high',label:'Высокий риск'},{value:'medium',label:'Средний риск'},{value:'low',label:'Низкий риск'},{value:'limited',label:'Ограниченные данные'},{value:'insufficient',label:'Недостаточно данных'}]}/></label></div>
    <div className="table-scroll"><table><thead><tr>{columns.map(([key,title])=><th key={key}><button onClick={()=>sortBy(key)}>{title}{sort===key?<ArrowDown size={12} style={{transform:direction===1?'rotate(180deg)':undefined}}/>:<ArrowUpDown size={12}/>}</button></th>)}<th/></tr></thead><tbody>{table.pageRows.map(r=><tr key={r.id}><td><Link className="client-link" to={`/snapshot?client=${r.id}`}><span className={`avatar avatar-${Number(r.id.slice(-1))%4}`}>{r.name.split(' ').map(n=>n[0]).join('')}</span><span><strong>{r.name}</strong><small>{r.id}</small></span></Link></td><td className="email">{r.email}</td><td className="numeric">{money(r.balanceCents)}</td><td className="numeric muted">{money(r.topupsCents)}</td><td>{r.level==='insufficient'?<span className="muted" title="Недостаточно данных для оценки">нет оценки</span>:<div className="score-cell"><b>{r.score}</b><div><span style={{width:`${r.score}%`,background:colors[r.level]}}/></div></div>}</td><td><Badge level={r.level}/></td><td><Link className="row-arrow" aria-label={`Анализ ${r.id}`} to={`/snapshot?client=${r.id}`}><ArrowUpRight size={17}/></Link></td></tr>)}</tbody></table>{!ordered.length&&<div className="table-empty">Клиенты не найдены. Измените поиск или фильтр.</div>}</div>
    <div className="pagination"><span>Показано {ordered.length?currentPage*limit+1:0}–{Math.min((currentPage+1)*limit,ordered.length)} из {ordered.length} клиентов</span><div><label>На странице <GlassSelect aria-label="Количество строк" value={String(limit)} onChange={value=>{setLimit(Number(value));setPage(0);}} options={[{value:'5',label:'5'},{value:'10',label:'10'}]}/></label><Button aria-label="Предыдущая страница" disabled={currentPage===0} onClick={()=>setPage(currentPage-1)}><ChevronLeft size={15}/></Button><span className="page-number">{currentPage+1}</span><Button aria-label="Следующая страница" disabled={currentPage>=totalPages-1} onClick={()=>setPage(currentPage+1)}><ChevronRight size={15}/></Button></div></div>
   </Panel>
  </div>
 );
}
