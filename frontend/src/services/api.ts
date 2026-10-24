import {createDataset,defaults} from '../mocks/data';
import type {Dataset,Settings} from '../types';
export interface AnalyzerApi { getData():Promise<Dataset>; getSettings():Promise<Settings>; saveSettings(settings:Settings):Promise<void>; saveData(data:Dataset):Promise<void> }
const pause=()=>new Promise(r=>setTimeout(r,300));
function read<T>(key:string,fallback:T):T {const raw=localStorage.getItem(key);return raw?JSON.parse(raw):fallback;}
export const mockApi:AnalyzerApi={
 async getData(){await pause();const d=read('aeza-data-v1',createDataset());if(!Array.isArray(d.accounts)||!Array.isArray(d.services)||!Array.isArray(d.transactions))throw new Error('Некорректные сохранённые данные');return d;},
 async getSettings(){await pause();const s=read('aeza-settings-v1',defaults);if(!s.weights||!Number.isFinite(s.highThreshold))throw new Error('Некорректные настройки');return s;},
 async saveSettings(settings){await pause();localStorage.setItem('aeza-settings-v1',JSON.stringify(settings));},
 async saveData(data){await pause();localStorage.setItem('aeza-data-v1',JSON.stringify(data));}
};
