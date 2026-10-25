import {createDataset,defaults} from '../mocks/data';
import type {Dataset,Settings} from '../types';
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
  weights:{paymentCoverage:weight('paymentCoverage','balance'),noAutoProlong:weight('noAutoProlong','autoRenew'),prolongWait:weight('prolongWait','status'),failedProlong:weight('failedProlong','transactions'),expiringRevenue:weight('expiringRevenue','renewal'),priceRisk:weight('priceRisk','priceRisk')},
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
