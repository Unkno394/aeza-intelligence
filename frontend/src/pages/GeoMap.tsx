import {useMemo,useState} from 'react';
import {geoNaturalEarth1,geoPath} from 'd3-geo';
import geoCsv from '../data/geoMap.csv?raw';
import world from '../data/world.json';
import countryCodes from '../data/countryCodes.json';

const brands=[
 {code:'aeza',name:'Aéza',color:'#ce514d'},
 {code:'hetzner',name:'Hetzner',color:'#6198ec'},
 {code:'contabo',name:'Contabo',color:'#e5ad48'},
 {code:'timeweb_cloud',name:'Timeweb Cloud',color:'#69ba9b'},
 {code:'selectel',name:'Selectel',color:'#a582d8'}
] as const;
type Country={code:string;name:string;scores:number[];winner:number};

const aliases:Record<string,string>={
 'Фолклендские (Мальвинские) о-ва':'FK',
 'Гонконг':'HK',
 'Макао':'MO',
 'Палестина':'PS',
 'Бонэйр, Синт-Эстатиус и Саба':'BQ'
 ,'Виргинские о-ва (Великобритания)':'VG'
 ,'о-в Св. Елены':'SH'
};

function csvRows(text:string):string[][]{
 const rows:string[][]=[];let row:string[]=[],cell='',quoted=false;
 for(let i=0;i<text.length;i++){
  const char=text[i];
  if(char==='"'){
   if(quoted&&text[i+1]==='"'){cell+='"';i++;}else quoted=!quoted;
  }else if(char===','&&!quoted){row.push(cell.trim());cell='';}
  else if((char==='\n'||char==='\r')&&!quoted){if(char==='\r'&&text[i+1]==='\n')i++;row.push(cell.trim());if(row.some(Boolean))rows.push(row);row=[];cell='';}
  else cell+=char;
 }
 row.push(cell.trim());if(row.some(Boolean))rows.push(row);
 return rows;
}

function parseGeoCsv(text:string):{countries:Country[];period:string}{
 const rows=csvRows(text);
 const headerIndex=rows.findIndex(row=>row[0]==='Страна'&&row.length===brands.length+1);
 if(headerIndex<0)throw new Error('В geoMap.csv не найдена таблица стран для пяти брендов');
 const header=rows[headerIndex];
 brands.forEach((brand,index)=>{if(!header[index+1].toLocaleLowerCase('ru').includes(brand.code==='aeza'?'aeza':brand.name.toLocaleLowerCase('ru')))throw new Error(`Неверная колонка ${brand.name} в geoMap.csv`);});
 const display=new Intl.DisplayNames(['ru'],{type:'region'});
 const codes=new Map<string,string>();
 for(let a=65;a<=90;a++)for(let b=65;b<=90;b++){
  const code=String.fromCharCode(a,b),name=display.of(code);
  if(name&&name!==code)codes.set(name.toLocaleLowerCase('ru'),code);
 }
 const countries:Country[]=[];
 for(const row of rows.slice(headerIndex+1)){
  if(row.length!==header.length||!row[0])continue;
  const scores=row.slice(1).map(value=>{
   const normalized=value.replace(/[\s\u00a0\u202f%]/g,'');
   if(!normalized)return 0;
   const score=Number(normalized.replace(',','.'));
   return Number.isFinite(score)&&score>=0&&score<=100?score:0;
  });
  if(!scores.some(score=>score>0))continue;
  const code=aliases[row[0]]??codes.get(row[0].toLocaleLowerCase('ru'));
  if(!code)continue;
  const winner=scores.indexOf(Math.max(...scores));
  countries.push({code,name:row[0],scores,winner});
 }
 return {countries,period:header[1].match(/\(([^)]+)\)/)?.[1]??''};
}

const data=parseGeoCsv(geoCsv);

export default function GeoMap({brand}:{brand:string}){
 const selected=brands.findIndex(item=>item.code===brand);
 const selectedIndex=selected<0?0:selected;
 const [hovered,setHovered]=useState<Country|null>(null);
 const byCode=useMemo(()=>new Map(data.countries.map(country=>[country.code,country])),[]);
 const paths=useMemo(()=>{
  const projection=geoNaturalEarth1().fitExtent([[18,24],[982,510]],world as never);
  const path=geoPath(projection);
  return world.features.map(feature=>({id:feature.id,shape:path(feature as never)??''}));
 },[]);
 const ranked=useMemo(()=>[...data.countries].filter(country=>country.scores[selectedIndex]>0).sort((a,b)=>b.scores[selectedIndex]-a.scores[selectedIndex]).slice(0,8),[selectedIndex]);
 return <div className="market-map-layout"><div className="market-map-wrap market-country-map-wrap">
  <svg className="market-country-map" viewBox="0 0 1000 540" role="img" aria-label={`Карта интереса к ${brands[selectedIndex].name} по странам`}>
   <rect width="1000" height="540" fill="#0b1927"/>
   {paths.map(({id,shape})=>{const country=byCode.get((countryCodes as Record<string,string>)[id]);const score=country?.scores[selectedIndex]??0;return <path key={id} d={shape} className={`market-country${score?' has-data':''}${hovered?.code===country?.code&&country?' is-hovered':''}`} fill={score?brands[selectedIndex].color:'#233547'} fillOpacity={score?0.28+0.72*Math.sqrt(score/100):1} onMouseEnter={()=>setHovered(country??null)} onMouseLeave={()=>setHovered(null)}><title>{country?`${country.name}: ${brands[selectedIndex].name} ${score}%`:id}</title></path>})}
  </svg>
  {hovered&&<div className="market-map-tooltip"><strong>{hovered.name}</strong><span>{brands[selectedIndex].name}: {hovered.scores[selectedIndex]}%</span><small>Лидер: {brands[hovered.winner].name} · {hovered.scores[hovered.winner]}%</small></div>}
 </div><aside className="market-map-aside">
  <div className="market-map-label">GOOGLE TRENDS · ПОПУЛЯРНОСТЬ ПО РЕГИОНАМ</div>
  <p>Чем ярче страна, тем выше доля {brands[selectedIndex].name} среди пяти запросов. Наведите на страну, чтобы увидеть её данные.</p>
  <div className="market-map-legend"><span><i style={{background:brands[selectedIndex].color}}/>Доля {brands[selectedIndex].name}</span><span><i style={{background:'#233547'}}/>Нет данных</span></div>
  {data.period&&<small>Период: {data.period}</small>}
  <ol>{ranked.map(country=><li key={country.code}><span>{country.name}</span><b>{country.scores[selectedIndex]}%</b></li>)}</ol>
 </aside></div>;
}
