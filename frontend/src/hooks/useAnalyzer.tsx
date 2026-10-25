import {createContext,useContext,useEffect,useRef,useState,type ReactNode} from 'react';
import {useAuth} from './useAuth';
import {LiveClient,liveUrl,type LiveStatus} from '../services/live';
import {applyLiveMessage,emptyAnalyzerLiveState} from '../services/liveState';
import type {Dataset,RiskResult,Settings,SettingsMetadata} from '../types';
interface State {data:Dataset|null;settings:Settings|null;settingsMetadata:SettingsMetadata|null;risks:Record<string,RiskResult>|null;connection:LiveStatus;loading:boolean;error:string;reload:()=>void;saveSettings:(s:Settings)=>Promise<SettingsMetadata>;saveData:(d:Dataset)=>Promise<void>}
const Context=createContext<State>(null!);
export function AnalyzerProvider({children}:{children:ReactNode}) {
 const [liveState,setLiveState]=useState(emptyAnalyzerLiveState);
 const [connection,setConnection]=useState<LiveStatus>('connecting'),[error,setError]=useState('');
 const client=useRef<LiveClient|null>(null),received=useRef(false);
 const {data,settings,settingsMetadata,risks}=liveState;
 const {recheck}=useAuth();
 useEffect(()=>{
  const live=new LiveClient(liveUrl('/api/client/ws'),message=>{
   if(message.type==='snapshot'){received.current=true;setError('');}
   if(message.type==='error')console.error('Live API:',message.message);
   setLiveState(current=>applyLiveMessage(current,message));
  },status=>{setConnection(status);// A closed channel may mean the session ended: the gate then shows the login screen.
   if(status==='closed')void recheck();
   if(status==='closed'&&!received.current)setError('Нет соединения с сервером (WebSocket /api/client/ws)');});
  client.current=live;live.start();
  return ()=>{live.stop();client.current=null;};
 },[]);
 function reload(){setError('');client.current?.stop();client.current?.start();}
 async function saveSettings(s:Settings){const metadata=await client.current!.updateSettings(s);setLiveState(current=>({...current,settings:s,settingsMetadata:metadata}));return metadata;}
 async function saveData(_d:Dataset):Promise<void>{throw new Error('Клиентские данные доступны только для чтения');}
 return <Context.Provider value={{data,settings,settingsMetadata,risks,connection,loading:!data&&!error,error,reload,saveSettings,saveData}}>{children}</Context.Provider>;
}
export const useAnalyzer=()=>useContext(Context);
