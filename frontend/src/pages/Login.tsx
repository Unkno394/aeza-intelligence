import { useEffect, useState, type FormEvent } from 'react';
import { ArrowRight, KeyRound, LockKeyhole, RefreshCw, ShieldCheck, TriangleAlert } from 'lucide-react';
import { localLogin, login } from '../services/auth';
import './login.css';

// Codes set by /api/auth/callback as ?authError=.
const errors: Record<string, string> = {
  forbidden: 'Этот email не найден среди активных администраторов.',
  unverified: 'Провайдер не подтвердил email.',
  expired: 'Вход занял слишком долго или был начат в другой вкладке. Попробуйте снова.',
  denied: 'Провайдер отклонил вход.',
  failed: 'Не удалось завершить вход через провайдера.',
};

type LoginStatus = 'loading' | 'anonymous' | 'error';

export default function Login({ status, error, retry, onSuccess }: { status: LoginStatus; error: string; retry: () => void; onSuccess: () => void }) {
  const [username,setUsername]=useState('admin');
  const [password,setPassword]=useState('');
  const [loginError,setLoginError]=useState('');
  const [busy,setBusy]=useState(false);
  const [authMethods,setAuthMethods]=useState<{sso:boolean;local:boolean}|null>(null);
  useEffect(()=>{
    const controller=new AbortController();
    let cancelled=false;
    const timeout=window.setTimeout(()=>controller.abort(),5000);
    // market_api has one way to sign in, auth.mode of config.toml.
    fetch('/api/auth/method',{signal:controller.signal})
      .then(response=>response.ok?response.json():null)
      .then(value=>{if(!cancelled)setAuthMethods({sso:value?.method==='oidc',local:value?.method==='password'});})
      .catch(()=>{if(!cancelled)setAuthMethods({sso:false,local:false});})
      .finally(()=>window.clearTimeout(timeout));
    return()=>{cancelled=true;window.clearTimeout(timeout);controller.abort();};
  },[]);
  async function submit(event:FormEvent){event.preventDefault();setBusy(true);setLoginError('');try{await localLogin(username,password);onSuccess();}catch(cause){setLoginError(cause instanceof Error?cause.message:String(cause));}finally{setBusy(false);}}
  useEffect(() => {
    try { document.documentElement.dataset.theme = localStorage.getItem('aeza-theme') === 'light' ? 'light' : 'dark'; } catch { /* storage may be disabled */ }
  }, []);

  const authError = new URLSearchParams(location.search).get('authError');
  const ssoEnabled=authMethods?.sso===true;
  const localEnabled=authMethods?.local===true;
  const redirect=status==='anonymous'&&ssoEnabled&&!localEnabled&&!authError;
  useEffect(()=>{if(redirect)login();},[redirect]);

  if(status==='loading'||(status==='anonymous'&&authMethods===null)||redirect){
    return <div className="loading-state"><RefreshCw className="spin" size={26}/><h2>{redirect?'Переходим ко входу':status==='loading'?'Проверяем сессию':'Проверяем способы входа'}</h2></div>;
  }

  return <main className="login-page">
    <div className="login-grid" aria-hidden="true" />
    <div className="login-glow login-glow-one" aria-hidden="true" />
    <div className="login-glow login-glow-two" aria-hidden="true" />

    <div className="login-content">
      <a className="login-brand" href="/" aria-label="Aéza Intelligence">
        <span className="login-brand-word">aéza<span>✳</span></span>
        <span className="login-brand-caption">INTELLIGENCE</span>
      </a>

      <section className="login-card" aria-labelledby="login-title">
        <div className="login-card-icon">{status === 'error' ? <TriangleAlert size={21}/> : <ShieldCheck size={21}/>}</div>
        <div className="login-eyebrow">ЗАЩИЩЁННОЕ РАБОЧЕЕ ПРОСТРАНСТВО</div>
        <h1 id="login-title">{status === 'error' ? 'Не удалось проверить вход' : 'Добро пожаловать'}</h1>
        <p className="login-description">Войдите, чтобы открыть аналитику клиентов и рынка.</p>

        {status === 'error' && <div className="login-error" role="alert">
          <strong>Сервис авторизации временно недоступен</strong>
          <span>{error || 'Сервер не ответил на проверку сессии.'}</span>
          <button type="button" className="login-retry" onClick={retry}>Повторить проверку <ArrowRight size={15}/></button>
        </div>}

        {status === 'anonymous' && <>
          {authError && <div className="login-auth-error" role="alert"><TriangleAlert size={16}/><span>{errors[authError] ?? 'Не удалось завершить вход. Попробуйте ещё раз.'}</span></div>}
          {loginError && <div className="login-auth-error" role="alert"><TriangleAlert size={16}/><span>{loginError}</span></div>}
          {localEnabled&&<form className="login-form" onSubmit={submit}>
            <label>Логин<input autoComplete="username" value={username} onChange={event=>setUsername(event.target.value)} required/></label>
            <label>Пароль<input type="password" autoComplete="current-password" value={password} onChange={event=>setPassword(event.target.value)} required/></label>
            <button type="submit" className="login-primary" disabled={busy}><KeyRound size={17}/><span>{busy?'Входим…':'Войти'}</span><ArrowRight size={16}/></button>
          </form>}
          {ssoEnabled&&<button type="button" className="login-primary" onClick={() => { history.replaceState(null, '', location.pathname); login(); }}>
            <KeyRound size={17}/><span>Войти через SSO</span><ArrowRight size={16}/>
          </button>}
          {!localEnabled&&!ssoEnabled&&<div className="login-auth-error" role="alert"><TriangleAlert size={16}/><span>Способ входа не настроен или API авторизации недоступен.</span></div>}
          <div className="login-security-note"><LockKeyhole size={14}/><span>Доступ защищён сессией</span></div>
        </>}
      </section>

      <footer className="login-footer"><span>AEZA INTELLIGENCE</span><i/><span>SECURE ACCESS</span></footer>
    </div>
  </main>;
}
