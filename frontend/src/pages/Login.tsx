import {useEffect} from 'react';
import {LogIn,RefreshCw,TriangleAlert} from 'lucide-react';
import {Button} from '../components/ui/button';
import {login} from '../services/auth';
// Codes set by /api/auth/callback as ?authError=.
const errors:Record<string,string>={forbidden:'Этот email не найден среди активных администраторов.',unverified:'Провайдер не подтвердил email.',expired:'Вход занял слишком долго или был начат в другой вкладке. Попробуйте снова.',denied:'Провайдер отклонил вход.',failed:'Не удалось завершить вход через провайдера.'};
export default function Login({status,error,retry}:{status:'loading'|'anonymous'|'error';error:string;retry:()=>void}){
 useEffect(()=>{try{document.documentElement.dataset.theme=localStorage.getItem('aeza-theme')==='light'?'light':'dark';}catch{}},[]);
 const authError=new URLSearchParams(location.search).get('authError');
 // Without a session go straight to the provider; the screen stays only to show a failed login, so it cannot loop.
 const redirect=status==='anonymous'&&!authError;
 useEffect(()=>{if(redirect)login();},[redirect]);
 if(status==='loading'||redirect)return <div className="loading-state"><RefreshCw className="spin" size={26}/><h2>{redirect?'Переходим ко входу':'Проверяем сессию'}</h2></div>;
 if(status==='error')return <div className="loading-state"><TriangleAlert size={28}/><h2>API недоступен</h2><p>Не удалось проверить сессию: {error||'нет ответа от сервера'}</p><Button onClick={retry}>Повторить</Button></div>;
 return <div className="loading-state"><h2>aéza intelligence</h2><p>Войдите через корпоративный аккаунт, чтобы открыть аналитику.</p>{authError&&<div className="error-box" role="alert">{errors[authError]??'Не удалось войти.'}</div>}<Button variant="default" onClick={()=>{history.replaceState(null,'',location.pathname);login();}}><LogIn size={15}/>Войти</Button></div>;
}
