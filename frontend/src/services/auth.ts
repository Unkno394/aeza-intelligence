// Session API of market_api: the login itself is a redirect through the OIDC provider.
export interface Admin {id:number;email:string;name:string;role:'viewer'|'operator'|'superadmin'}
export const canEditSettings=(admin:Admin|null)=>admin?.role==='operator'||admin?.role==='superadmin';
/** The signed-in admin, or null when there is no live session. */
export async function fetchMe():Promise<Admin|null>{const r=await fetch('/api/auth/me');if(r.status===401)return null;if(!r.ok)throw new Error(r.status===502?'market_api не отвечает (502). Проверьте docker compose logs api.':`API: ${r.status}`);return r.json();}
export function login(){const returnTo=location.pathname+location.search;location.assign(`/api/auth/login?returnTo=${encodeURIComponent(returnTo)}`);}
export async function logout(){const r=await fetch('/api/auth/logout',{method:'POST'});const body=r.ok?await r.json() as {logoutUrl:string|null}:{logoutUrl:null};location.assign(body.logoutUrl??'/');}
