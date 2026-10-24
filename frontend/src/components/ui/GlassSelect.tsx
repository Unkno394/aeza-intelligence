import {useEffect,useId,useRef,useState,type KeyboardEvent,type PointerEvent} from 'react';
import {createPortal} from 'react-dom';
import {Check,ChevronDown} from 'lucide-react';
import './glass-select.css';

export interface GlassSelectOption {value:string;label:string;disabled?:boolean}
interface GlassSelectProps {
 value:string;
 options:GlassSelectOption[];
 onChange:(value:string)=>void;
 'aria-label'?:string;
 className?:string;
 disabled?:boolean;
 placeholder?:string;
}
interface Position {left:number;top:number;width:number}

export function GlassSelect({value,options,onChange,className='',disabled=false,placeholder='Выберите…',...ariaProps}:GlassSelectProps){
 const rootRef=useRef<HTMLDivElement>(null);
 const triggerRef=useRef<HTMLButtonElement>(null);
 const menuRef=useRef<HTMLDivElement>(null);
 const optionRefs=useRef<(HTMLDivElement|null)[]>([]);
 const menuId=useId();
 const [open,setOpen]=useState(false);
 const [activeIndex,setActiveIndex]=useState(0);
 const [position,setPosition]=useState<Position>({left:0,top:0,width:180});
 const selectedIndex=options.findIndex(option=>option.value===value);
 const selected=selectedIndex>=0?options[selectedIndex]:undefined;

 const openMenu=()=>{
  const firstEnabled=options.findIndex(option=>!option.disabled);
  setActiveIndex(selectedIndex>=0&&!options[selectedIndex]?.disabled?selectedIndex:Math.max(0,firstEnabled));
  setOpen(true);
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
  if(!options.length)return;
  let next=activeIndex;
  for(let count=0;count<options.length;count++){
   next=(next+direction+options.length)%options.length;
   if(!options[next]?.disabled)break;
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

 useEffect(()=>{
  if(!open)return;
  const updatePosition=()=>{
   const rect=triggerRef.current?.getBoundingClientRect();
   if(!rect)return;
   const width=Math.min(Math.max(rect.width,180),window.innerWidth-16);
   const left=Math.max(8,Math.min(rect.left,window.innerWidth-width-8));
   const estimatedHeight=Math.min(280,options.length*40+12);
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
 },[open,options.length]);

 const onOptionPointerMove=(event:PointerEvent<HTMLDivElement>,index:number)=>{
  if(event.pointerType==='mouse')setActiveIndex(index);
 };

 return <div className={`glass-select ${className}`.trim()} ref={rootRef}>
  <button ref={triggerRef} type="button" role="combobox" className="glass-select-trigger" aria-haspopup="listbox" aria-expanded={open} aria-controls={open?menuId:undefined} aria-activedescendant={open&&options[activeIndex]?`${menuId}-option-${activeIndex}`:undefined} disabled={disabled} onClick={()=>open?closeMenu():openMenu()} onKeyDown={onTriggerKeyDown} {...ariaProps}>
   <span className="glass-select-value">{selected?.label??placeholder}</span><ChevronDown size={15} className="glass-select-chevron" aria-hidden="true"/>
  </button>
  {open&&createPortal(<div ref={menuRef} id={menuId} className="glass-select-menu" role="listbox" aria-label={ariaProps['aria-label']} style={{left:position.left,top:position.top,width:position.width}}>
   {options.map((option,index)=><div key={option.value} id={`${menuId}-option-${index}`} ref={element=>{optionRefs.current[index]=element;}} role="option" aria-selected={option.value===value} aria-disabled={option.disabled||undefined} className={`glass-select-option${index===activeIndex?' is-active':''}${option.value===value?' is-selected':''}${option.disabled?' is-disabled':''}`} onPointerMove={event=>onOptionPointerMove(event,index)} onClick={()=>choose(option)}>
    <span>{option.label}</span>{option.value===value&&<Check size={15} aria-hidden="true"/>}
   </div>)}
  </div>,document.body)}
 </div>;
}
