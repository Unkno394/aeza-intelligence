import type {ReactNode} from 'react';
import type {RiskLevel} from '../types';
export const labels={low:'Низкий',medium:'Средний',high:'Высокий',limited:'Ограниченные данные',insufficient:'Недостаточно данных'};
export const colors={low:'var(--risk-low)',medium:'var(--risk-medium)',high:'var(--risk-high)',limited:'var(--text-muted)',insufficient:'var(--text-muted)'};
export function Badge({level}:{level:RiskLevel}) {return <span className={`badge ${level}`}><i/>{labels[level]}</span>;}
export function Panel({title,subtitle,action,children,className=''}:{title:string;subtitle?:string;action?:ReactNode;children:ReactNode;className?:string}){return <section className={`panel ${className}`}><div className="panel-heading"><div><h2>{title}</h2>{subtitle&&<p>{subtitle}</p>}</div>{action}</div>{children}</section>;}
export function Empty({icon,title,children}:{icon:ReactNode;title:string;children:ReactNode}){return <div className="empty"><div className="empty-icon">{icon}</div><h2>{title}</h2><p>{children}</p></div>;}
