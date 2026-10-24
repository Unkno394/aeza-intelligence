import {useEffect,useState,type CSSProperties} from 'react';

type Tile={current:string;next:string;flipping:boolean;tick:number};
type SplitFlapTextProps={
 text:string;
 charset?:string;
 fontSize?:number;
 gap?:number;
 tileColor?:string;
 textColor?:string;
 tileRadius?:number;
 flipDuration?:number;
 stagger?:number;
 flipsPerChar?:number;
 repeatEvery?:number;
 intro?:boolean;
 className?:string;
};

const randomGlyph=(charset:string)=>charset[Math.floor(Math.random()*charset.length)]??' ';

export default function SplitFlapText({text,charset='abcdefghijklmnopqrstuvwxyzé',fontSize=13,gap=1,tileColor='#142b39',textColor='#d9f2ff',tileRadius=2,flipDuration=.11,stagger=.025,flipsPerChar=5,repeatEvery=30000,intro=true,className=''}:SplitFlapTextProps){
 const [tiles,setTiles]=useState<Tile[]>(()=>Array.from(text,character=>({current:intro?' ':character,next:intro?' ':character,flipping:false,tick:0})));
 useEffect(()=>{
  let cancelled=false;
  let hasPlayed=false;
  const timers:ReturnType<typeof setTimeout>[]=[];
  const clearAnimation=()=>{timers.forEach(clearTimeout);timers.length=0;};
  const reduceMotion=window.matchMedia?.('(prefers-reduced-motion: reduce)').matches??false;
  if(reduceMotion){setTiles(Array.from(text,character=>({current:character,next:character,flipping:false,tick:0})));return()=>{cancelled=true;};}
  const flipMs=Math.max(40,flipDuration*1000);
  const play=()=>{
   clearAnimation();
   const firstFrame=intro&&!hasPlayed;
   hasPlayed=true;
   const initial=Array.from(text,character=>firstFrame?' ':character);
   setTiles(initial.map(character=>({current:character,next:character,flipping:false,tick:0})));
   Array.from(text).forEach((target,index)=>{
    const sequence=Array.from({length:Math.max(0,flipsPerChar)},()=>randomGlyph(charset));
    sequence.push(target);
    sequence.forEach((next,step)=>{
     const delay=index*stagger*1000+step*(flipMs+14);
     timers.push(setTimeout(()=>{
      if(cancelled)return;
      setTiles(previous=>previous.map((tile,tileIndex)=>tileIndex===index?{...tile,next,flipping:true,tick:tile.tick+1}:tile));
      timers.push(setTimeout(()=>{if(!cancelled)setTiles(previous=>previous.map((tile,tileIndex)=>tileIndex===index?{current:next,next,flipping:false,tick:tile.tick}:tile));},flipMs));
     },delay));
    });
   });
  };
  play();
  const interval=repeatEvery>0?window.setInterval(play,repeatEvery):undefined;
  return()=>{cancelled=true;if(interval!==undefined)window.clearInterval(interval);clearAnimation();};
 },[text,charset,flipDuration,stagger,flipsPerChar,repeatEvery,intro]);

 const style={
  '--split-flap-font-size':`${fontSize}px`,
  '--split-flap-gap':`${gap}px`,
  '--split-flap-tile-color':tileColor,
  '--split-flap-text-color':textColor,
  '--split-flap-radius':`${tileRadius}px`,
  '--split-flap-flip-duration':`${flipDuration}s`,
 } as CSSProperties;
 return <span className={`split-flap-text ${className}`} style={style} role="img" aria-label={text}>{tiles.map((tile,index)=><span className="split-flap-tile" aria-hidden="true" key={index}>
  <span className="split-flap-half split-flap-top"><span className="split-flap-glyph">{tile.flipping?tile.next:tile.current}</span></span>
  <span className="split-flap-half split-flap-bottom"><span className="split-flap-glyph">{tile.current}</span></span>
  {tile.flipping&&<><span className="split-flap-flap split-flap-front" key={`front-${tile.tick}`}><span className="split-flap-glyph">{tile.current}</span></span><span className="split-flap-flap split-flap-back" key={`back-${tile.tick}`}><span className="split-flap-glyph">{tile.next}</span></span></>}
 </span>)}</span>;
}
