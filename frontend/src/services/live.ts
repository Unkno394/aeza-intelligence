import type {Dataset,RiskResult,Settings} from '../types';
// Protocol of market_api /api/client/ws: the server pushes data and Risk Score changes, the client sends settings.
export type AccountRisk=RiskResult&{accountId:string};
export type ServerMessage=
 |{type:'snapshot';dataset:Dataset;settings:Settings;risks:AccountRisk[]}
 |{type:'dataset';dataset:Dataset}
 |{type:'settings';settings:Settings}
 |{type:'scores';changed:AccountRisk[];removed:string[]}
 |{type:'ack';requestId:string}
 |{type:'error';requestId:string|null;message:string};
export type LiveStatus='connecting'|'open'|'closed';
export const liveUrl=(path:string)=>`${location.protocol==='https:'?'wss':'ws'}://${location.host}${path}`;
export class LiveClient {
 private socket:WebSocket|null=null;private retries=0;private timer=0;private seq=0;
 private pending=new Map<string,{resolve:()=>void;reject:(error:Error)=>void}>();
 constructor(private url:string,private onMessage:(message:ServerMessage)=>void,private onStatus:(status:LiveStatus)=>void){}
 start(){clearTimeout(this.timer);this.retries=0;this.connect();}
 stop(){clearTimeout(this.timer);const socket=this.socket;this.socket=null;socket?.close();this.failPending('Соединение закрыто');}
 updateSettings(settings:Settings){return new Promise<void>((resolve,reject)=>{const socket=this.socket;if(socket?.readyState!==WebSocket.OPEN)return reject(new Error('Нет соединения с сервером'));const requestId=String(++this.seq);this.pending.set(requestId,{resolve,reject});socket.send(JSON.stringify({type:'updateSettings',requestId,settings}));});}
 private connect(){
  const socket=new WebSocket(this.url);this.socket=socket;this.onStatus('connecting');
  socket.onopen=()=>{this.retries=0;this.onStatus('open');};
  socket.onmessage=event=>{let message:ServerMessage;try{message=JSON.parse(event.data);}catch{return;}
   if(message.type==='ack'||message.type==='error'&&message.requestId){const id=message.requestId!,request=this.pending.get(id);this.pending.delete(id);if(message.type==='ack')request?.resolve();else request?.reject(new Error(message.message));return;}
   this.onMessage(message);};
  // A stopped or replaced socket must not reconnect.
  socket.onclose=()=>{if(this.socket!==socket)return;this.socket=null;this.failPending('Соединение с сервером потеряно');this.onStatus('closed');this.timer=window.setTimeout(()=>this.connect(),Math.min(30000,1000*2**this.retries++));};
 }
 private failPending(reason:string){for(const request of this.pending.values())request.reject(new Error(reason));this.pending.clear();}
}
