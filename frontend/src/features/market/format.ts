/** Display precision only; never round API payloads or ledger arithmetic. */
const decimal = new Intl.NumberFormat('ru-RU', {maximumFractionDigits: 2});
export function number2(value: number|string|null|undefined): string {
  if (value === null || value === undefined || value === '') return '—';
  const n=Number(value);return Number.isFinite(n)?decimal.format(Object.is(n,-0)?0:n):'—';
}
export function input2(value:number):number {return Number(value.toFixed(2));}
export function metadataDisplay(value:unknown):string {
 return JSON.stringify(value,(_key,v:unknown)=>{
  if(typeof v==='number')return input2(v);
  if(typeof v==='string'&&/^-?\d+\.\d+$/.test(v))return Number(v).toFixed(2);
  return v;
 },2);
}
