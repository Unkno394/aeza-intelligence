import {createDataset,defaults} from '../mocks/data';
import type {Dataset,RiskResult,Settings} from '../types';
export interface AnalyzerApi { getData():Promise<Dataset>; getSettings():Promise<Settings>; saveSettings(settings:Settings):Promise<void>; saveData(data:Dataset):Promise<void> }
const pause=()=>new Promise(r=>setTimeout(r,300));
function read<T>(key:string,fallback:T):T {const raw=localStorage.getItem(key);return raw?JSON.parse(raw):fallback;}
function normalizeSettings(value:unknown):Settings {
 if(!value||typeof value!=='object')return structuredClone(defaults);
 const saved=value as Partial<Settings>&{weights?:Record<string,unknown>};
 const old:Record<string,unknown>=saved.weights??{};
 const weight=(name:string,legacy:string)=>{const raw=old[name]??old[legacy]??defaults.weights[name as keyof Settings['weights']];return typeof raw==='number'&&Number.isFinite(raw)?Math.min(100,Math.max(0,raw)):defaults.weights[name as keyof Settings['weights']];};
 return {
  ...defaults,...saved,
  weights:{paymentCoverage:weight('paymentCoverage','balance'),noAutoProlong:weight('noAutoProlong','autoRenew'),prolongWait:weight('prolongWait','status'),failedProlong:weight('failedProlong','transactions'),expiringRevenue:weight('expiringRevenue','renewal')},
  mediumThreshold:typeof saved.mediumThreshold==='number'?saved.mediumThreshold:defaults.mediumThreshold,
  highThreshold:typeof saved.highThreshold==='number'?saved.highThreshold:defaults.highThreshold,
  renewalWindowDays:typeof saved.renewalWindowDays==='number'?saved.renewalWindowDays:defaults.renewalWindowDays,
  // HistoricalModel's server-side minimum profile span is currently 30 days.
  historicalMinDays:defaults.historicalMinDays,
 };
}
function readSettings():Settings {try{return normalizeSettings(read<unknown>('aeza-settings-v1',defaults));}catch{return structuredClone(defaults);}}
export const mockApi:AnalyzerApi={
 async getData(){await pause();const d=read('aeza-data-v1',createDataset());if(!Array.isArray(d.accounts)||!Array.isArray(d.services)||!Array.isArray(d.transactions))throw new Error('Некорректные сохранённые данные');return d;},
 async getSettings(){await pause();return readSettings();},
 async saveSettings(settings){await pause();localStorage.setItem('aeza-settings-v1',JSON.stringify(settings));},
 async saveData(data){await pause();localStorage.setItem('aeza-data-v1',JSON.stringify(data));}
};
// REST adapter implements this contract; the UI does not depend on mock storage.
export class HttpApi implements AnalyzerApi {
 constructor(private baseUrl:string){}
 private async request<T>(path:string,init?:RequestInit):Promise<T>{const r=await fetch(`${this.baseUrl}${path}`,{...init,headers:{'Content-Type':'application/json',...init?.headers}});if(!r.ok)throw new Error(`API: ${r.status}`);return r.status===204?undefined as T:r.json();}
 getData(){return this.request<Dataset>('/dataset');}
 async getRisks(settings:Settings){const rows=await this.request<(RiskResult&{accountId:string})[]>('/snapshot',{method:'POST',body:JSON.stringify(settings)});return Object.fromEntries(rows.map(({accountId,...risk})=>[accountId,risk])) as Record<string,RiskResult>;}
 async getSettings(){return readSettings();}
 async saveSettings(s:Settings){localStorage.setItem('aeza-settings-v1',JSON.stringify(s));}
 async saveData(_d:Dataset):Promise<void>{throw new Error('Клиентские данные доступны только для чтения');}
}
