import type {RiskLevel} from '../../types';

export interface CustomerTableRow {
  id: string;
  name: string;
  email: string;
  balanceCents: number;
  topupsCents: number;
  score: number;
  level: RiskLevel;
}

export type CustomerSortKey = 'id'|'email'|'balanceCents'|'topupsCents'|'score'|'level';
export interface CustomerTableOptions {
  search: string;
  risk: RiskLevel|'all';
  sort: CustomerSortKey;
  direction: 1|-1;
  page: number;
  limit: number;
}

export function selectCustomerRows<T extends CustomerTableRow>(rows: readonly T[], options: CustomerTableOptions) {
  const search=options.search.trim().toLocaleLowerCase();
  const ordered=rows
    .map((row,index)=>({row,index}))
    .filter(({row})=>(options.risk==='all'||row.level===options.risk)
      &&(!search||`${row.id} ${row.email} ${row.name}`.toLocaleLowerCase().includes(search)))
    .sort((left,right)=>{
      const a=left.row,b=right.row;
      let comparison=0;
      if(options.sort==='id'&&/^-?\d+$/.test(a.id)&&/^-?\d+$/.test(b.id)){
        const x=BigInt(a.id),y=BigInt(b.id);comparison=x<y?-1:x>y?1:0;
      }else{
        const x=a[options.sort],y=b[options.sort];
        comparison=typeof x==='number'&&typeof y==='number'?x-y:String(x).localeCompare(String(y));
      }
      return comparison===0?left.index-right.index:comparison*options.direction;
    })
    .map(({row})=>row);
  const limit=Number.isSafeInteger(options.limit)&&options.limit>0?options.limit:1;
  const totalPages=Math.max(1,Math.ceil(ordered.length/limit));
  const currentPage=Math.min(Math.max(0,Number.isSafeInteger(options.page)?options.page:0),totalPages-1);
  return {
    ordered,
    pageRows:ordered.slice(currentPage*limit,currentPage*limit+limit),
    totalPages,
    currentPage,
  };
}
