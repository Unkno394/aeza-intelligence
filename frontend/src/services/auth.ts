// Session API of market_api. auth.mode of config.toml decides the one way to sign in:
// a password form ("password") or a redirect through the OIDC provider ("oidc").
export type LoginMethod='password'|'oidc'|'disabled';
export interface Admin {id:number;email:string;name:string;role:'viewer'|'operator'|'superadmin'}
export const canEditSettings=(admin:Admin|null)=>admin?.role==='operator'||admin?.role==='superadmin';
/** The signed-in admin, or null when there is no live session. */
export async function fetchMe():Promise<Admin|null>{const r=await fetch('/api/auth/me');if(r.status===401)return null;if(!r.ok)throw new Error(r.status===502?'market_api не отвечает (502). Проверьте docker compose logs api.':`API: ${r.status}`);return r.json();}
/** How admins sign in; the login page offers only this. */
export async function fetchLoginMethod():Promise<LoginMethod>{const r=await fetch('/api/auth/method');if(!r.ok)throw new Error(r.status===502?'market_api не отвечает (502). Проверьте docker compose logs api.':`API: ${r.status}`);return (await r.json() as {method:LoginMethod}).method;}
export function login(){const returnTo=location.pathname+location.search;location.assign(`/api/auth/login?returnTo=${encodeURIComponent(returnTo)}`);}
export async function localLogin(username:string,password:string){const response=await fetch('/api/auth/login',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({username,password})});if(response.status===401)throw new Error('Неверный логин или пароль');if(!response.ok)throw new Error(`Не удалось войти: API ${response.status}`);}
export async function logout(){const r=await fetch('/api/auth/logout',{method:'POST'});const body=r.ok?await r.json() as {logoutUrl:string|null}:{logoutUrl:null};location.assign(body.logoutUrl??'/');}
