import type {Plan} from './api';

export type MarketObservation = {
  providerCode: string;
  provider: string;
  planKey: string;
  planName: string;
  cpuCores: number|null;
  ramGib: number|null;
  diskGib: number|null;
  diskType: string|null;
  region: string|null;
  sourceKind?: 'archive'|'live'|string;
  observedOn: string;
  effectiveAt?: string|null;
  retrievedAt?: string;
  priceRub: number|null;
  priceUsd?: number|null;
  priceEur?: number|null;
  rubRateOn?: string|null;
  usdRateOn?: string|null;
  eurRateOn?: string|null;
  previousPriceRub?: number|null;
  billingPeriod: string|null;
  vatIncluded?: boolean|null;
  priceKind?: string;
  sourceUrl: string;
};

export type MarketMatch = {
  observation: MarketObservation;
  status: 'exact'|'approximate'|'incompatible';
  plan: Plan|null;
  reasons: string[];
};

const normalize=(value:string)=>value.toLocaleLowerCase().normalize('NFKD').replace(/[\u0300-\u036f]/g,'').replace(/[^\p{L}\p{N}]+/gu,'');
const providerKey=(code:string,name:string)=>{
  const key=normalize(code||name);
  const aliases:Record<string,string>={timeweb:'timewebcloud',timewebcloud:'timewebcloud',firstvds:'firstvds',hetzner:'hetzner',aeza:'aeza',contabo:'contabo',selectel:'selectel'};
  return aliases[key]??key;
};
const regionKey=(value:string)=>{
  const key=normalize(value);
  const aliases:Record<string,string>={moscow:'moscow',moscowru:'moscow',moskva:'moscow',москва:'moscow',russia:'russia',ru:'russia',россия:'russia',frankfurt:'frankfurt',frankfurtam:'frankfurt',warsaw:'warsaw',singapore:'singapore',amsterdam:'amsterdam',london:'london',paris:'paris'};
  return aliases[key]??key;
};
const close=(a:number,b:number,tolerance:number)=>Math.abs(a-b)<=Math.max(Math.abs(a),Math.abs(b),1)*tolerance;
const monthly=(plan:Plan)=>{
  const unit=normalize(plan.details?.original_price.unit??'');
  return unit==='month'||unit==='monthly';
};
const planRegions=(plan:Plan)=>[...new Set([plan.details?.source_region??'',...plan.regions].filter(Boolean).map(regionKey))];

/**
 * Match PostgreSQL observations against the simulator snapshot using provider,
 * CPU/RAM/disk, region and monthly billing. Approximate rows are never eligible
 * to create scenario events.
 */
export function matchMarketObservations(observations:MarketObservation[],catalog:Plan[]):MarketMatch[]{
  return observations.map(observation=>{
    const provider=providerKey(observation.providerCode,observation.provider);
    const sameProvider=catalog.filter(plan=>providerKey(plan.provider,plan.provider)===provider);
    const monthlyObservation=observation.billingPeriod==='month';
    const configKnown=observation.cpuCores!==null&&observation.ramGib!==null&&observation.diskGib!==null;
    const region=observation.region?regionKey(observation.region):null;
    const evaluated=sameProvider.map(plan=>{
      const resourcesKnown=configKnown&&plan.vcpu>0&&plan.ram_gb>0&&plan.disk_gb>0;
      // PostgreSQL stores RAM and disk in GiB; simulator offers are normalized to GB.
      const ramGb=observation.ramGib===null?null:observation.ramGib*1.073741824;
      const diskGb=observation.diskGib===null?null:observation.diskGib*1.073741824;
      const cpuExact=resourcesKnown&&close(observation.cpuCores!,plan.vcpu,.01);
      const ramExact=resourcesKnown&&ramGb!==null&&close(ramGb,plan.ram_gb,.01);
      const diskExact=resourcesKnown&&diskGb!==null&&close(diskGb,plan.disk_gb,.01);
      const configExact=cpuExact&&ramExact&&diskExact;
      const configNear=resourcesKnown&&ramGb!==null&&diskGb!==null&&close(observation.cpuCores!,plan.vcpu,.15)&&close(ramGb,plan.ram_gb,.15)&&close(diskGb,plan.disk_gb,.15);
      const regions=planRegions(plan);
      const regionExact=region!==null&&regions.includes(region);
      const regionUnknown=region===null||regions.length===0;
      const termExact=monthlyObservation&&monthly(plan);
      return {plan,configExact,configNear,regionExact,regionUnknown,termExact};
    });
    const exact=evaluated.filter(item=>item.configExact&&item.regionExact&&item.termExact);
    const preferred=exact.length===1?exact:evaluated.filter(item=>item.configNear&&(item.regionExact||item.regionUnknown)&&item.termExact);
    const best=preferred.length===1?preferred[0]:null;
    if(best){
      const exactMatch=best.configExact&&best.regionExact&&best.termExact;
      return {observation,status:exactMatch?'exact':'approximate',plan:best.plan,reasons:exactMatch
        ?['Провайдер совпадает','CPU, RAM и диск совпадают после перевода GiB в GB','Регион совпадает','Период оплаты — месяц','Связь однозначна в каталоге']
        :['Провайдер совпадает',best.regionExact?'Регион совпадает':'Регион не подтверждён в каталоге',best.configExact?'Характеристики CPU, RAM и диска совпадают':'Ресурсы близки, но не идентичны','Период оплаты — месяц','Приблизительную связь нельзя применить к сценарию']};
    }
    if(sameProvider.length===0){
      return {observation,status:'incompatible',plan:null,reasons:['Провайдер не найден в каталоге симулятора']};
    }
    const reasons:string[]=[];
    if(!monthlyObservation)reasons.push(`Период оплаты ${observation.billingPeriod??'неизвестен'}; нужен месяц`);
    else if(!sameProvider.some(plan=>monthly(plan)))reasons.push('Период оплаты в каталоге не подтверждён как месячный');
    if(!configKnown)reasons.push('Не хватает CPU, RAM или диска в наблюдении');
    else if(!sameProvider.some(plan=>plan.vcpu>0&&plan.ram_gb>0&&plan.disk_gb>0))reasons.push('Не хватает конфигурации в каталоге симулятора');
    else if(!sameProvider.some(plan=>close(observation.cpuCores!,plan.vcpu,.15)&&close(observation.ramGib!*1.073741824,plan.ram_gb,.15)&&close(observation.diskGib!*1.073741824,plan.disk_gb,.15)))reasons.push('CPU, RAM или диск отличаются больше чем на 15%');
    if(!region)reasons.push('Регион наблюдения неизвестен');
    else if(!sameProvider.some(plan=>planRegions(plan).includes(region)))reasons.push(`Регион «${observation.region}» не подтверждён в каталоге`);
    if(exact.length>1)reasons.push('Несколько тарифов имеют одинаковые характеристики; соответствие неоднозначно');
    else if(!exact.length&&sameProvider.some(plan=>planRegions(plan).length===0))reasons.push('В каталоге не указан регион части тарифов');
    return {observation,status:'incompatible',plan:null,reasons:reasons.length?reasons:['Нет однозначного соответствия по провайдеру, конфигурации, региону и периоду оплаты']};
  });
}

export function matchStatusLabel(status:MarketMatch['status']):string{
  return status==='exact'?'Точное совпадение':status==='approximate'?'Приблизительное':'Несовместимо';
}
