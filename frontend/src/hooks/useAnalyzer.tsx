import {createContext,useContext,useEffect,useRef,useState,type ReactNode} from 'react';
import {LiveClient,liveUrl,type AccountRisk,type LiveStatus} from '../services/live';
import type {Dataset,RiskResult,Settings} from '../types';
interface State {data:Dataset|null;settings:Settings|null;risks:Record<string,RiskResult>|null;connection:LiveStatus;loading:boolean;error:string;reload:()=>void;saveSettings:(s:Settings)=>Promise<void>;saveData:(d:Dataset)=>Promise<void>}
const Context=createContext<State>(null!);
const byAccount=(rows:AccountRisk[])=>Object.fromEntries(rows.map(({accountId,...risk})=>[accountId,risk])) as Record<string,RiskResult>;
export function AnalyzerProvider({children}:{children:ReactNode}) {
 const [data,setData]=useState<Dataset|null>(null),[settings,setSettings]=useState<Settings|null>(null),[risks,setRisks]=useState<Record<string,RiskResult>|null>(null);
 const [connection,setConnection]=useState<LiveStatus>('connecting'),[error,setError]=useState('');
 const client=useRef<LiveClient|null>(null),received=useRef(false);
 useEffect(()=>{
  const live=new LiveClient(liveUrl('/api/client/ws'),message=>{switch(message.type){
   case 'snapshot':received.current=true;setData(message.dataset);setSettings(message.settings);setRisks(byAccount(message.risks));setError('');break;
   case 'dataset':setData(message.dataset);break;
   case 'settings':setSettings(message.settings);break;
   case 'scores':setRisks(current=>{const next={...current};for(const id of message.removed)delete next[id];return {...next,...byAccount(message.changed)};});break;
   case 'error':console.error('Live API:',message.message);break;
  }},status=>{setConnection(status);if(status==='closed'&&!received.current)setError('Нет соединения с сервером (WebSocket /api/client/ws)');});
  client.current=live;live.start();
  return ()=>{live.stop();client.current=null;};
 },[]);
 function reload(){setError('');client.current?.stop();client.current?.start();}
 async function saveSettings(s:Settings){await client.current!.updateSettings(s);setSettings(s);}
 async function saveData(_d:Dataset):Promise<void>{throw new Error('Клиентские данные доступны только для чтения');}
 return <Context.Provider value={{data,settings,risks,connection,loading:!data&&!error,error,reload,saveSettings,saveData}}>{children}</Context.Provider>;
}
export const useAnalyzer=()=>useContext(Context);
