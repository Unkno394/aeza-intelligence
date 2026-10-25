import { useEffect, useState, type FormEvent } from 'react';
import { ArrowRight, KeyRound, LoaderCircle, LockKeyhole, ShieldCheck, TriangleAlert } from 'lucide-react';
import { fetchLoginMethod, localLogin, login, type LoginMethod } from '../services/auth';
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
  // The one way to sign in, from auth.mode of config.toml; null until it is known.
  const [method,setMethod]=useState<LoginMethod|null>(null);
  const [methodError,setMethodError]=useState('');
  useEffect(()=>{if(status!=='anonymous')return;fetchLoginMethod().then(setMethod).catch(cause=>setMethodError(cause instanceof Error?cause.message:String(cause)));},[status]);
  async function submit(event:FormEvent){event.preventDefault();setBusy(true);setLoginError('');try{await localLogin(username,password);onSuccess();}catch(cause){setLoginError(cause instanceof Error?cause.message:String(cause));}finally{setBusy(false);}}
  useEffect(() => {
    try { document.documentElement.dataset.theme = localStorage.getItem('aeza-theme') === 'light' ? 'light' : 'dark'; } catch { /* storage may be disabled */ }
  }, []);

  const authError = new URLSearchParams(location.search).get('authError');

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

        {status === 'loading' && <div className="login-state" role="status"><LoaderCircle className="login-spinner" size={18}/><span>Проверяем сессию…</span></div>}

        {status === 'error' && <div className="login-error" role="alert">
          <strong>Сервис авторизации временно недоступен</strong>
          <span>{error || 'Сервер не ответил на проверку сессии.'}</span>
          <button type="button" className="login-retry" onClick={retry}>Повторить проверку <ArrowRight size={15}/></button>
        </div>}

        {status === 'anonymous' && <>
          {authError && <div className="login-auth-error" role="alert"><TriangleAlert size={16}/><span>{errors[authError] ?? 'Не удалось завершить вход. Попробуйте ещё раз.'}</span></div>}
          {loginError && <div className="login-auth-error" role="alert"><TriangleAlert size={16}/><span>{loginError}</span></div>}
          {methodError && <div className="login-error" role="alert">
            <strong>Не удалось узнать способ входа</strong>
            <span>{methodError}</span>
            <button type="button" className="login-retry" onClick={()=>{setMethodError('');fetchLoginMethod().then(setMethod).catch(cause=>setMethodError(cause instanceof Error?cause.message:String(cause)));}}>Повторить <ArrowRight size={15}/></button>
          </div>}
          {!method && !methodError && <div className="login-state" role="status"><LoaderCircle className="login-spinner" size={18}/><span>Загружаем способ входа…</span></div>}
          {method === 'password' && <form className="login-form" onSubmit={submit}>
            <label>Логин<input autoComplete="username" value={username} onChange={event=>setUsername(event.target.value)} required/></label>
            <label>Пароль<input type="password" autoComplete="current-password" value={password} onChange={event=>setPassword(event.target.value)} required/></label>
            <button type="submit" className="login-primary" disabled={busy}><KeyRound size={17}/><span>{busy?'Входим…':'Войти'}</span><ArrowRight size={16}/></button>
          </form>}
          {method === 'oidc' && <button type="button" className="login-primary" onClick={() => { history.replaceState(null, '', location.pathname); login(); }}>
            <KeyRound size={17}/><span>Войти через SSO</span><ArrowRight size={16}/>
          </button>}
          <div className="login-security-note"><LockKeyhole size={14}/><span>Доступ защищён сессией</span></div>
        </>}
      </section>

      <footer className="login-footer"><span>AEZA INTELLIGENCE</span><i/><span>SECURE ACCESS</span></footer>
    </div>
  </main>;
}
