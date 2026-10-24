import {useEffect,useId,useRef,useState,type KeyboardEvent,type PointerEvent} from 'react';
import {createPortal} from 'react-dom';
import {Check,ChevronDown,Search} from 'lucide-react';
import './glass-select.css';

export interface GlassSelectOption {value:string;label:string;disabled?:boolean;searchText?:string}
interface GlassSelectProps {
 value:string;
 options:GlassSelectOption[];
 onChange:(value:string)=>void;
 'aria-label'?:string;
 className?:string;
 disabled?:boolean;
 placeholder?:string;
 searchable?:boolean;
}
interface Position {left:number;top:number;width:number}

export function GlassSelect({value,options,onChange,className='',disabled=false,placeholder='Выберите…',searchable=false,...ariaProps}:GlassSelectProps){
 const rootRef=useRef<HTMLDivElement>(null);
 const triggerRef=useRef<HTMLButtonElement>(null);
 const menuRef=useRef<HTMLDivElement>(null);
 const searchRef=useRef<HTMLInputElement>(null);
 const optionRefs=useRef<(HTMLDivElement|null)[]>([]);
 const menuId=useId();
 const [open,setOpen]=useState(false);
 const [simulatorMenu,setSimulatorMenu]=useState(false);
 const [query,setQuery]=useState('');
 const [activeIndex,setActiveIndex]=useState(0);
 const [position,setPosition]=useState<Position>({left:0,top:0,width:180});
 const selectedIndex=options.findIndex(option=>option.value===value);
 const selected=selectedIndex>=0?options[selectedIndex]:undefined;
 const normalizedQuery=query.trim().toLocaleLowerCase();
 const visibleOptions=searchable&&normalizedQuery?options.filter(option=>(option.searchText??option.label).toLocaleLowerCase().includes(normalizedQuery)):options;

 const openMenu=()=>{
  setSimulatorMenu(Boolean(rootRef.current?.closest('.simulator-page')));
  const firstEnabled=options.findIndex(option=>!option.disabled);
  setActiveIndex(selectedIndex>=0&&!options[selectedIndex]?.disabled?selectedIndex:Math.max(0,firstEnabled));
  setQuery('');
  setOpen(true);
  if(searchable)requestAnimationFrame(()=>searchRef.current?.focus());
 };
 const closeMenu=(restoreFocus=false)=>{
  setOpen(false);
  if(restoreFocus)requestAnimationFrame(()=>triggerRef.current?.focus());
 };
 const choose=(option:GlassSelectOption)=>{
  if(option.disabled)return;
  onChange(option.value);
  closeMenu(true);
 };
 const moveActive=(direction:number)=>{
  if(!visibleOptions.length)return;
  let next=activeIndex;
  for(let count=0;count<visibleOptions.length;count++){
   next=(next+direction+visibleOptions.length)%visibleOptions.length;
   if(!visibleOptions[next]?.disabled)break;
  }
  setActiveIndex(next);
  requestAnimationFrame(()=>optionRefs.current[next]?.scrollIntoView({block:'nearest'}));
 };
 const onTriggerKeyDown=(event:KeyboardEvent<HTMLButtonElement>)=>{
  if(event.key==='ArrowDown'||event.key==='ArrowUp'){
   event.preventDefault();
   if(open)moveActive(event.key==='ArrowDown'?1:-1);
   else openMenu();
  }else if((event.key==='Home'||event.key==='End')&&open){
   event.preventDefault();
   const candidates=options.map((option,index)=>({option,index})).filter(item=>!item.option.disabled);
   if(candidates.length)setActiveIndex(event.key==='Home'?candidates[0].index:candidates.at(-1)!.index);
  }else if((event.key==='Enter'||event.key===' ')&&open){
   event.preventDefault();const option=options[activeIndex];if(option)choose(option);
  }else if(event.key==='Escape'&&open){event.preventDefault();closeMenu(true);}
  else if(event.key==='Tab'&&open)closeMenu();
 };
 const onSearchKeyDown=(event:KeyboardEvent<HTMLInputElement>)=>{
  if(event.key==='ArrowDown'||event.key==='ArrowUp'){
   event.preventDefault();moveActive(event.key==='ArrowDown'?1:-1);
  }else if(event.key==='Enter'){
   event.preventDefault();const option=visibleOptions[activeIndex];if(option)choose(option);
  }else if((event.key==='Home'||event.key==='End')&&visibleOptions.length){
   event.preventDefault();
   const candidates=visibleOptions.map((option,index)=>({option,index})).filter(item=>!item.option.disabled);
   if(candidates.length)setActiveIndex(event.key==='Home'?candidates[0].index:candidates.at(-1)!.index);
  }else if(event.key==='Escape'){event.preventDefault();closeMenu(true);}
  else if(event.key==='Tab')closeMenu();
 };

 useEffect(()=>{
  if(!open)return;
  const updatePosition=()=>{
   const rect=triggerRef.current?.getBoundingClientRect();
   if(!rect)return;
   const width=Math.min(Math.max(rect.width,180),window.innerWidth-16);
   const left=Math.max(8,Math.min(rect.left,window.innerWidth-width-8));
   const estimatedHeight=Math.min(320,visibleOptions.length*40+(searchable?58:12));
   const below=window.innerHeight-rect.bottom-8;
   const above=rect.top-8;
   const placeAbove=below<estimatedHeight&&above>below;
   const top=placeAbove?Math.max(8,rect.top-Math.min(estimatedHeight,above)-8):Math.min(rect.bottom+6,window.innerHeight-estimatedHeight-8);
   setPosition({left,top,width});
  };
  const handlePointerDown=(event:globalThis.PointerEvent)=>{
   const target=event.target as Node;
   if(!rootRef.current?.contains(target)&&!menuRef.current?.contains(target))closeMenu();
  };
  updatePosition();
  window.addEventListener('resize',updatePosition);
  window.addEventListener('scroll',updatePosition,true);
  document.addEventListener('pointerdown',handlePointerDown,true);
  return()=>{
   window.removeEventListener('resize',updatePosition);
   window.removeEventListener('scroll',updatePosition,true);
   document.removeEventListener('pointerdown',handlePointerDown,true);
  };
 },[open,options.length,searchable,visibleOptions.length]);

 const onOptionPointerMove=(event:PointerEvent<HTMLDivElement>,index:number)=>{
  if(event.pointerType==='mouse')setActiveIndex(index);
 };
 const optionList=visibleOptions.length?visibleOptions.map((option,index)=><div key={option.value} id={`${menuId}-option-${index}`} ref={element=>{optionRefs.current[index]=element;}} role="option" aria-selected={option.value===value} aria-disabled={option.disabled||undefined} className={`glass-select-option${index===activeIndex?' is-active':''}${option.value===value?' is-selected':''}${option.disabled?' is-disabled':''}`} onPointerMove={event=>onOptionPointerMove(event,index)} onClick={()=>choose(option)}>
  <span>{option.label}</span>{option.value===value&&<Check size={15} aria-hidden="true"/>}
 </div>):<div className="glass-select-empty">Нет вариантов по запросу</div>;

 return <div className={`glass-select ${className}`.trim()} ref={rootRef}>
  <button ref={triggerRef} type="button" role="combobox" className="glass-select-trigger" aria-haspopup={searchable?'dialog':'listbox'} aria-expanded={open} aria-controls={open?menuId:undefined} aria-activedescendant={open&&visibleOptions[activeIndex]?`${menuId}-option-${activeIndex}`:undefined} disabled={disabled} onClick={()=>open?closeMenu():openMenu()} onKeyDown={onTriggerKeyDown} {...ariaProps}>
   <span className="glass-select-value">{selected?.label??placeholder}</span><ChevronDown size={15} className="glass-select-chevron" aria-hidden="true"/>
  </button>
  {open&&createPortal(<div ref={menuRef} id={menuId} className={`glass-select-menu${simulatorMenu?' simulator-select-menu':''}`} role={searchable?'dialog':'listbox'} aria-label={ariaProps['aria-label']} style={{left:position.left,top:position.top,width:position.width}}>
   {searchable&&<label className="glass-select-search"><Search size={14} aria-hidden="true"/><input ref={searchRef} type="search" value={query} onChange={event=>{setQuery(event.target.value);setActiveIndex(0);}} onKeyDown={onSearchKeyDown} placeholder="Поиск по имени, почте или ID…" aria-label="Поиск в списке" aria-controls={`${menuId}-options`} aria-activedescendant={visibleOptions[activeIndex]?`${menuId}-option-${activeIndex}`:undefined} /></label>}
   {searchable?<div id={`${menuId}-options`} className="glass-select-options" role="listbox" aria-label={ariaProps['aria-label']}>{optionList}</div>:optionList}
  </div>,document.body)}
 </div>;
}
