import http from 'node:http';
import {createReadStream, statSync} from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

const root=path.join(path.dirname(fileURLToPath(import.meta.url)),'dist');
const mime={'.html':'text/html; charset=utf-8','.js':'text/javascript; charset=utf-8','.css':'text/css; charset=utf-8','.svg':'image/svg+xml','.png':'image/png','.jpg':'image/jpeg','.webp':'image/webp','.ico':'image/x-icon','.json':'application/json','.woff2':'font/woff2'};

function proxy(req,res,target){
 const upstream=new URL(req.url,target);
 const outgoing=http.request(upstream,{method:req.method,headers:{...req.headers,host:upstream.host}},response=>{
  res.writeHead(response.statusCode??502,response.headers);
  response.pipe(res);
 });
 outgoing.setTimeout(120000,()=>outgoing.destroy(new Error('API timeout')));
 outgoing.on('error',error=>{
  if(!res.headersSent)res.writeHead(502,{'content-type':'text/plain; charset=utf-8'});
  res.end(`API unavailable: ${error.message}`);
 });
 req.pipe(outgoing);
}

// WebSocket upgrade (/api/client/ws): forward the handshake, then pipe both sockets.
function proxyUpgrade(req,socket,head,target){
 const upstream=new URL(req.url,target);
 const outgoing=http.request(upstream,{method:req.method,headers:{...req.headers,host:upstream.host}});
 outgoing.on('upgrade',(response,upstreamSocket,upstreamHead)=>{
  const headers=[];
  for(let i=0;i<response.rawHeaders.length;i+=2)headers.push(`${response.rawHeaders[i]}: ${response.rawHeaders[i+1]}`);
  socket.write(`HTTP/1.1 101 ${response.statusMessage}\r\n${headers.join('\r\n')}\r\n\r\n`);
  if(upstreamHead.length)socket.write(upstreamHead);
  if(head.length)upstreamSocket.write(head);
  upstreamSocket.on('error',()=>socket.destroy());
  socket.on('error',()=>upstreamSocket.destroy());
  upstreamSocket.pipe(socket).pipe(upstreamSocket);
 });
 outgoing.on('response',response=>{socket.end(`HTTP/1.1 ${response.statusCode} ${response.statusMessage}\r\nconnection: close\r\n\r\n`);response.resume();});
 outgoing.on('error',()=>socket.destroy());
 socket.on('error',()=>outgoing.destroy());
 outgoing.end();
}

const API='http://host.docker.internal:3000';

// Market Simulator has no auth of its own: let a request through only with a live market_api session.
function authorized(req){
 return new Promise(resolve=>{
  const check=http.request(new URL('/api/auth/me',API),{headers:{cookie:req.headers.cookie??''}},response=>{response.resume();resolve(response.statusCode===200);});
  check.setTimeout(10000,()=>check.destroy());
  check.on('error',()=>resolve(false));
  check.end();
 });
}

function serve(req,res){
 const pathname=new URL(req.url,'http://localhost').pathname;
 if(pathname.startsWith('/api/market/v1/')||pathname.startsWith('/api/market/v2/'))return void authorized(req).then(ok=>{if(ok)return proxy(req,res,'http://simulator:8091');res.writeHead(401,{'content-type':'application/json'});res.end('{"error":"unauthorized"}');});
 if(pathname.startsWith('/api/'))return proxy(req,res,API);
 let file;
 try{file=path.resolve(root,`.${decodeURIComponent(pathname)}`);}catch{res.writeHead(400);res.end();return;}
 if(!file.startsWith(`${root}${path.sep}`)&&file!==root){res.writeHead(403);res.end();return;}
 let info;
 try{info=statSync(file);}catch{}
 if(!info?.isFile())file=path.join(root,'index.html');
 try{info=statSync(file);}catch{res.writeHead(404);res.end();return;}
 res.writeHead(200,{'content-type':mime[path.extname(file)]??'application/octet-stream','content-length':info.size,'cache-control':file.endsWith('index.html')?'no-cache':'public, max-age=3600'});
 if(req.method==='HEAD')res.end();else createReadStream(file).pipe(res);
}

const server=http.createServer(serve);
server.on('upgrade',(req,socket,head)=>{
 if(new URL(req.url,'http://localhost').pathname==='/api/client/ws')return proxyUpgrade(req,socket,head,API);
 socket.destroy();
});
server.listen(80,'0.0.0.0',()=>console.log('Frontend listening on port 80'));
