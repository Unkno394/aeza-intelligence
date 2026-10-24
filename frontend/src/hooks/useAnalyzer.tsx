import {createContext,useContext,useEffect,useState,type ReactNode} from 'react';
import {HttpApi} from '../services/api';
import type {Dataset,Settings} from '../types';
interface State {data:Dataset|null;settings:Settings|null;loading:boolean;error:string;reload:()=>void;saveSettings:(s:Settings)=>Promise<void>;saveData:(d:Dataset)=>Promise<void>}
const Context=createContext<State>(null!);
const api=new HttpApi('/api/client');
export function AnalyzerProvider({children}:{children:ReactNode}) {
 const [data,setData]=useState<Dataset|null>(null),[settings,setSettings]=useState<Settings|null>(null),[loading,setLoading]=useState(true),[error,setError]=useState('');
 async function reload(){setLoading(true);setError('');try{const [d,s]=await Promise.all([api.getData(),api.getSettings()]);setData(d);setSettings(s);}catch(e){setError(e instanceof Error?e.message:'Ошибка загрузки');}finally{setLoading(false);}}
 useEffect(()=>{void reload();},[]);
 return <Context.Provider value={{data,settings,loading,error,reload,saveSettings:async s=>{await api.saveSettings(s);setSettings(s);},saveData:async d=>{await api.saveData(d);setData(d);}}}>{children}</Context.Provider>;
}
export const useAnalyzer=()=>useContext(Context);
