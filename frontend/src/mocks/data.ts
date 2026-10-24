import type { Dataset, Settings } from '../types';
export const defaults: Settings = {weights:{paymentCoverage:35,noAutoProlong:15,prolongWait:15,failedProlong:10,expiringRevenue:25},mediumThreshold:35,highThreshold:65,renewalWindowDays:14,historicalMinDays:30};
const names = ['Alex Morgan','Sofia Weber','Dmitry Volkov','Emma Wilson','Lucas Martin','Anna Schmidt','Oliver Brown','Maria Garcia','Noah Fischer','Kate Anderson'];
const balances = [420,18200,850,7200,120,31500,2250,5800,990,12300];
export function createDataset(): Dataset {
 const asOf='2026-10-23T12:00:00.000Z';
 const accounts=names.map((name,i)=>({id:`AE-${1041+i}`,name,email:`${name.toLowerCase().replace(' ','.')}@${['gmail.com','proton.me','outlook.com'][i%3]}`,balanceCents:balances[i],joinedAt:`2026-0${i%8+1}-12T12:00:00Z`}));
 const services=accounts.flatMap((a,i)=>Array.from({length:3},(_,j)=>({id:`S-${i*3+j+1}`,accountId:a.id,name:['VPS · Ryzen 4','VPS · EPYC 2','Storage · 100 GB'][j],region:['Frankfurt, DE','Helsinki, FI','Amsterdam, NL'][(i+j)%3],priceCents:[1490,890,490][j],renewsAt:new Date(Date.parse(asOf)+(i%3===0?j+1:i*3+j+3)*86400000).toISOString(),autoRenew:i%3===1||j===2,status:(i%4===0&&j===1?'suspended':i===8&&j===0?'cancelled':'active') as 'active'|'suspended'|'cancelled'})));
 const transactions=accounts.flatMap((a,i)=>Array.from({length:10},(_,j)=>({id:`TX-${String(i*10+j+1).padStart(4,'0')}`,accountId:a.id,...(j%3?{serviceId:services[i*3+j%3].id}:{}),amountCents:j%3?services[i*3+j%3].priceCents:[5000,10000,2500][i%3],type:(j%3?'payment':'topup') as 'payment'|'topup',status:(i%3===0&&j>6?'failed':'completed') as 'failed'|'completed',createdAt:new Date(Date.parse(asOf)-(j*3+i%3)*86400000).toISOString()})));
 return {accounts,services,transactions,asOf,snapshots:[]};
}
