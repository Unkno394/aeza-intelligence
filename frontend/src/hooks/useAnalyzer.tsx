import {createContext,useContext,useEffect,useRef,useState,type ReactNode} from 'react';
import {useAuth} from './useAuth';
import {LiveClient,liveUrl,type AccountRisk,type LiveStatus} from '../services/live';
import type {Dataset,RiskResult,Settings,SettingsMetadata} from '../types';
interface State {data:Dataset|null;settings:Settings|null;settingsMetadata:SettingsMetadata|null;risks:Record<string,RiskResult>|null;connection:LiveStatus;loading:boolean;error:string;reload:()=>void;saveSettings:(s:Settings)=>Promise<SettingsMetadata>;saveData:(d:Dataset)=>Promise<void>}
const Context=createContext<State>(null!);
const byAccount=(rows:AccountRisk[])=>Object.fromEntries(rows.map(({accountId,...risk})=>[accountId,risk])) as Record<string,RiskResult>;
export function AnalyzerProvider({children}:{children:ReactNode}) {
 const [data,setData]=useState<Dataset|null>(null),[settings,setSettings]=useState<Settings|null>(null),[settingsMetadata,setSettingsMetadata]=useState<SettingsMetadata|null>(null),[risks,setRisks]=useState<Record<string,RiskResult>|null>(null);
 const [connection,setConnection]=useState<LiveStatus>('connecting'),[error,setError]=useState('');
 const client=useRef<LiveClient|null>(null),received=useRef(false);
 const {recheck}=useAuth();
 useEffect(()=>{
  const live=new LiveClient(liveUrl('/api/client/ws'),message=>{switch(message.type){
   case 'snapshot':received.current=true;setData(message.dataset);setSettings(message.settings);setSettingsMetadata(message.settingsMetadata);setRisks(byAccount(message.risks));setError('');break;
   case 'dataset':setData(message.dataset);break;
   case 'settings':setSettings(message.settings);setSettingsMetadata(message.settingsMetadata);break;
   case 'scores':setRisks(current=>{const next={...current};for(const id of message.removed)delete next[id];return {...next,...byAccount(message.changed)};});break;
   case 'error':console.error('Live API:',message.message);break;
  }},status=>{setConnection(status);// A closed channel may mean the session ended: the gate then shows the login screen.
   if(status==='closed')void recheck();
   if(status==='closed'&&!received.current)setError('Нет соединения с сервером (WebSocket /api/client/ws)');});
  client.current=live;live.start();
  return ()=>{live.stop();client.current=null;};
 },[]);
 function reload(){setError('');client.current?.stop();client.current?.start();}
 async function saveSettings(s:Settings){const metadata=await client.current!.updateSettings(s);setSettings(s);setSettingsMetadata(metadata);return metadata;}
 async function saveData(_d:Dataset):Promise<void>{throw new Error('Клиентские данные доступны только для чтения');}
 return <Context.Provider value={{data,settings,settingsMetadata,risks,connection,loading:!data&&!error,error,reload,saveSettings,saveData}}>{children}</Context.Provider>;
}
export const useAnalyzer=()=>useContext(Context);
