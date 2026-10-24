import type {Account,Dataset,Settings,RiskResult} from '../../types';
export function calculateRisk(account: Account, data: Dataset, config: Settings): RiskResult {
 const services=data.services.filter(s=>s.accountId===account.id&&s.status!=='cancelled');
 const tx=data.transactions.filter(t=>t.accountId===account.id);
 const asOf=Date.parse(data.asOf),windowEnd=asOf+config.renewalWindowDays*86400000;
 const upcomingCents=services.reduce((sum,s)=>{
  const renewalAt=Date.parse(s.renewsAt);
  if(!Number.isFinite(renewalAt)||renewalAt>windowEnd)return sum;
  const termMs=(s.paymentTermSeconds??30*86400)*1000;
  const count=Number.isFinite(termMs)&&termMs>0?Math.max(1,Math.ceil((windowEnd-Math.max(renewalAt,asOf))/termMs)):1;
  return sum+count*(s.renewalPriceCents??s.priceCents);
 },0);
 const monthlyCents=services.reduce((sum,s)=>sum+s.priceCents,0);
 const recent=tx.filter(t=>Date.parse(t.createdAt)>=Date.parse(data.asOf)-30*86400000);
 const failures=recent.filter(t=>t.status==='failed').length;
 const values = [
 {key:'balance' as const,label:'Покрытие балансом',severity:upcomingCents?Math.max(0,1-account.balanceCents/upcomingCents):0,explanation:`Баланс ${account.balanceCents} центов; продления в окне — ${upcomingCents} центов. max(0, 1 − баланс / продления).`},
 {key:'renewal' as const,label:'Близость продления',severity:services.length?Math.max(0,...services.map(s=>1-Math.max(0,(Date.parse(s.renewsAt)-Date.parse(data.asOf))/86400000)/config.renewalWindowDays)):0,explanation:`Ближайшее продление: max(0, 1 − дней до продления / ${config.renewalWindowDays}).`},
 {key:'autoRenew' as const,label:'Автопродление отключено',severity:services.length?services.filter(s=>!s.autoRenew).length/services.length:0,explanation:'Доля услуг с отключённым автопродлением среди незакрытых услуг.'},
 {key:'status' as const,label:'Приостановленные услуги',severity:services.length?services.filter(s=>s.status==='suspended').length/services.length:0,explanation:'Доля приостановленных услуг среди незакрытых услуг.'},
 {key:'transactions' as const,label:'Неуспешные транзакции',severity:recent.length?failures/recent.length:0,explanation:`${failures} из ${recent.length} транзакций за 30 дней завершились ошибкой.`}
 ];
 const total=Object.values(config.weights).reduce((a,b)=>a+b,0)||1;
 const factors=values.map(f=>({...f,points:f.severity*config.weights[f.key]/total*100}));
 const score=Math.min(100,Math.max(0,Math.round(factors.reduce((s,f)=>s+f.points,0))));
 return {score,level:score>=config.highThreshold?'high':score>=config.mediumThreshold?'medium':'low',factors,upcomingCents,monthlyCents,topupsCents:account.totalReplenishedCents??tx.filter(t=>t.type==='topup'&&t.status==='completed').reduce((s,t)=>s+t.amountCents,0)};
}
