interface Props { active?: boolean; payload?: readonly {name?: string | number; value?: string | number; color?: string; payload?: {name?: string}}[]; label?: string | number }
export function ChartTooltip({active,payload,label}:Props){
 if(!active||!payload?.length)return null;
 const item=payload[0];
 return <div className="chart-tooltip" role="status"><div className="chart-tooltip-title"><span style={{background:item.color??'#a5d8ff'}}/>{item.payload?.name??label??item.name}</div><div className="chart-tooltip-value"><strong>{item.value}</strong><span>клиентов</span></div></div>;
}
