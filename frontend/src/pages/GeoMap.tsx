export type GeoInterest={brandCode:string;source:string;regionCode:string;regionName:string;latitude:number|null;longitude:number|null;periodStart:string;periodEnd:string;score:number};

const land=[
 'M71 82 114 53 166 48 196 66 209 95 242 106 253 143 226 167 199 158 181 183 151 186 135 165 102 154 87 128 62 114Z',
 'M183 190 213 197 233 224 234 270 256 294 247 327 222 370 207 359 199 318 181 289 177 242Z',
 'M310 72 334 53 369 59 386 76 379 97 357 104 340 90Z',
 'M380 124 415 105 462 98 489 114 504 141 488 166 464 172 454 203 431 224 402 205 395 170 373 151Z',
 'M486 80 540 58 606 54 646 66 688 53 747 59 802 79 839 107 829 138 803 153 765 145 740 176 709 173 686 201 651 191 629 162 594 154 569 182 542 163 518 141 490 119Z',
 'M700 252 739 232 790 233 817 253 833 286 803 309 755 311 720 293Z',
 'M835 311 849 307 854 322 844 337 835 328Z',
 'M450 339 503 329 548 340 594 347 637 341 684 351 712 373 667 393 602 395 545 389 484 394 433 378Z'
];

function coordinate(latitude:number,longitude:number){return {x:(longitude+180)/360*900,y:(90-latitude)/180*450};}

export default function GeoMap({points,brand,brandName}:{points:GeoInterest[];brand:string;brandName:string}){
 const latest=points.filter(point=>point.brandCode===brand).reduce((value,point)=>point.periodEnd>value?point.periodEnd:value,'');
 const visible=points.filter(point=>point.periodEnd===latest&&point.brandCode===brand&&Number.isFinite(point.latitude)&&Number.isFinite(point.longitude)&&Number.isFinite(point.score));
 const ranked=points.filter(point=>point.periodEnd===latest&&point.brandCode===brand).sort((a,b)=>b.score-a.score).slice(0,5);
 return <div className="market-map-layout"><div className="market-map-wrap">
 <svg className="market-map" viewBox="0 0 900 450" role="img" aria-label={visible.length?`Карта относительного интереса: ${visible.length} регионов`:'Карта ожидает географические данные'}>
 <defs><radialGradient id="market-sea"><stop stopColor="#153b57"/><stop offset="1" stopColor="#081a2d"/></radialGradient><radialGradient id="market-dot"><stop stopColor="#d9f5ff"/><stop offset=".38" stopColor="#9ddcff"/><stop offset="1" stopColor="#4fa9e8"/></radialGradient></defs>
 <rect width="900" height="450" rx="18" fill="url(#market-sea)"/>
 {[90,180,270,360].map(y=><path key={`h${y}`} d={`M0 ${y}H900`} stroke="#b9e4ff" strokeOpacity=".08"/>)}
 {[150,300,450,600,750].map(x=><path key={`v${x}`} d={`M${x} 0V450`} stroke="#b9e4ff" strokeOpacity=".08"/>)}
 {land.map((d,index)=><path key={index} d={d} fill="#31576c" fillOpacity=".78" stroke="#80b7cc" strokeOpacity=".35" strokeWidth="1.5"/>)}
 {visible.map(point=>{const {x,y}=coordinate(point.latitude!,point.longitude!);const radius=5+Math.sqrt(Math.max(0,Math.min(100,point.score))/100)*17;return <g key={`${point.regionCode}-${point.periodStart}`}><circle cx={x} cy={y} r={radius+7} fill="#75c8ff" opacity=".11"/><circle cx={x} cy={y} r={radius} fill="url(#market-dot)" opacity=".86" stroke="#e2f6ff" strokeWidth="1.4"><title>{`${point.regionName}: ${point.score.toFixed(1)} / 100`}</title></circle></g>;})}
 </svg>
 {!ranked.length&&<div className="market-map-empty"><strong>Нет данных по странам для {brandName}</strong><span>Загрузите CSV «Interest by region» из совместного сравнения Google Trends. TrendsMCP возвращает временной ряд без географии.</span></div>}
 </div><aside className="market-map-aside"><div className="market-map-label">{ranked.length?'GOOGLE TRENDS · ОТНОСИТЕЛЬНЫЙ ИНТЕРЕС':'ОТНОСИТЕЛЬНЫЙ ИНТЕРЕС'}</div><p>Размер круга соответствует индексу 0–100 из одного сравнения пяти брендов.</p>{latest&&<small>Период до {latest}</small>}
 {ranked.length?<ol>{ranked.map(point=><li key={point.regionCode}><span>{point.regionName}</span><b>{point.score.toFixed(0)}</b></li>)}</ol>:<div className="market-map-placeholder">После импорта CSV здесь появятся страны и их индексы.</div>}</aside></div>;
}
