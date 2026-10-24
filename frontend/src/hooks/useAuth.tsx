import {createContext,useContext,useEffect,useState,type ReactNode} from 'react';
import {fetchMe,type Admin} from '../services/auth';
import Login from '../pages/Login';
interface State {admin:Admin|null;recheck:()=>Promise<void>}
const Context=createContext<State>(null!);
/** Renders children only for a signed-in admin; otherwise the login screen. */
export function AuthGate({children}:{children:ReactNode}) {
 const [admin,setAdmin]=useState<Admin|null>(null),[status,setStatus]=useState<'loading'|'anonymous'|'ready'|'error'>('loading'),[error,setError]=useState('');
 async function load(){try{const me=await fetchMe();setAdmin(me);setStatus(me?'ready':'anonymous');}catch(e){setError(e instanceof Error?e.message:String(e));setStatus('error');}}
 // A failed recheck (e.g. API restart) keeps the current state; only a 401 signs out.
 async function recheck(){try{const me=await fetchMe();setAdmin(me);if(!me)setStatus('anonymous');}catch{}}
 useEffect(()=>{void load();},[]);
 if(status!=='ready')return <Login status={status} error={error} retry={()=>{setStatus('loading');void load();}} onSuccess={()=>{setStatus('loading');void load();}}/>;
 return <Context.Provider value={{admin,recheck}}>{children}</Context.Provider>;
}
export const useAuth=()=>useContext(Context);
