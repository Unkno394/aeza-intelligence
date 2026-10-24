import type {Account,Dataset,Settings,RiskResult} from '../../types';

/** Local calculation kept for fixture previews; the live UI uses the Rust API. */
export function calculateRisk(account: Account, data: Dataset, config: Settings): RiskResult {
 const services=data.services.filter(s=>s.accountId===account.id&&s.status!=='cancelled');
 const tx=data.transactions.filter(t=>t.accountId===account.id);
 const asOf=Date.parse(data.asOf),windowEnd=asOf+config.renewalWindowDays*86400000;
 const due=services.filter(s=>{const at=Date.parse(s.renewsAt);return Number.isFinite(at)&&at>asOf&&at<=windowEnd;});
 const upcomingCents=due.reduce((sum,s)=>{
  const termMs=(s.paymentTermSeconds??30*86400)*1000;
  const count=Number.isFinite(termMs)&&termMs>0?Math.max(1,Math.ceil((windowEnd-Math.max(Date.parse(s.renewsAt),asOf))/termMs)):1;
  return sum+count*(s.renewalPriceCents??s.priceCents);
 },0);
 const monthlyCents=services.reduce((sum,s)=>sum+s.priceCents,0);
 const expiringCents=due.reduce((sum,s)=>sum+s.priceCents,0);
 const failures=tx.filter(t=>t.status==='failed').length;
 const paymentCoverage=upcomingCents>0?Math.min(10,account.balanceCents/upcomingCents):null;
 const values:RiskResult['factors']=[
  {key:'noAutoProlong',label:'Услуги без автопродления',value:services.length?services.filter(s=>!s.autoRenew).length/services.length:null,available:services.length>0,severity:0,points:0,explanation:'Доля действующих услуг без автопродления.'},
  {key:'prolongWait',label:'Услуги в prolong_wait',value:services.length?services.filter(s=>s.status==='prolong_wait').length/services.length:null,available:services.length>0,severity:0,points:0,explanation:'Доля услуг, ожидающих продления.'},
  {key:'failedProlong',label:'Подтверждённые неуспешные продления',value:failures?failures/Math.max(1,tx.filter(t=>t.billingType==='prolong'||t.type==='payment').length):null,available:failures>0,severity:0,points:0,explanation:'Неизвестные исходы продления не считаются отказом.'},
  {key:'expiringRevenue',label:'Выручка по скоро истекающим услугам',value:monthlyCents?expiringCents/monthlyCents:null,available:monthlyCents>0,severity:0,points:0,explanation:'Доля месячной выручки от услуг, истекающих в окне анализа.'},
  {key:'paymentCoverage',label:'Покрытие ближайших платежей',value:paymentCoverage,available:paymentCoverage!==null,severity:0,points:0,explanation:'Рассчитывается только если в окне есть определённые платежи.'},
 ];
 for(const factor of values)factor.severity=factor.value===null?0:factor.key==='paymentCoverage'?Math.max(0,1-Math.min(1,factor.value)):Math.min(1,Math.max(0,factor.value));
 const weights=values.reduce((sum,f)=>sum+(f.available?config.weights[f.key]:0),0)||1;
 for(const factor of values)factor.points=factor.available?factor.severity*config.weights[factor.key]/weights*100:0;
 const score=Math.min(100,Math.max(0,Math.round(values.reduce((sum,f)=>sum+f.points,0))));
 const availableFactors=values.filter(f=>f.available).length;
 const assessmentStatus=services.length===0?'insufficient_data':paymentCoverage===null||availableFactors<3?'limited_data':'sufficient_data';
 const level=services.length===0?'insufficient':assessmentStatus==='limited_data'?'limited':score>=config.highThreshold?'high':score>=config.mediumThreshold?'medium':'low';
 return {score,level,assessmentStatus,availableFactors,factors:values,upcomingCents,monthlyCents,topupsCents:account.totalReplenishedCents??tx.filter(t=>t.type==='topup'&&t.status==='completed').reduce((s,t)=>s+t.amountCents,0),spendableBalanceCents:account.balanceCents,paymentCoverage};
}
