import http from 'node:http';
import https from 'node:https';
import {execFileSync} from 'node:child_process';
import {createReadStream, existsSync, mkdirSync, readFileSync, statSync} from 'node:fs';
import net, {isIP} from 'node:net';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {parse as parseToml} from 'smol-toml';

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

// The same path on server.public_url. Only the path and the query come from the
// request: a target like //evil.example or http://evil.example/ must not choose
// where the redirect goes.
function redirectToPublicUrl(req,res){
 const requested=new URL(req.url,'http://request.invalid');
 // Assigned, not concatenated: a path such as //evil.example stays a path.
 const location=new URL(publicUrl.origin);
 location.pathname=requested.pathname;
 location.search=requested.search;
 res.writeHead(308,{location:location.href});
 res.end();
}

function serve(req,res){
 // Cookies and the API's Origin check belong to server.public_url: another name for
 // this host (localhost instead of 127.0.0.1, the machine's IP) is sent there. It
 // also keeps a DNS-rebinding page (a foreign Host header) from reading responses.
 if(req.headers.host!==publicUrl.host)return redirectToPublicUrl(req,res);
 const pathname=new URL(req.url,'http://localhost').pathname;
 if(pathname.startsWith('/api/market/v1/')||pathname.startsWith('/api/market/v2/'))return void authorized(req).then(ok=>{if(ok)return proxy(req,res,'http://simulator:8091');res.writeHead(401,{'content-type':'application/json'});res.end('{"error":"unauthorized"}');});
 // /metrics is market_api's Prometheus endpoint, outside /api like in the v1 contract.
 if(pathname.startsWith('/api/')||pathname==='/metrics')return proxy(req,res,API);
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

// server.public_url of the shared config.toml (AEZA_CONFIG). With https:// the
// frontend serves HTTPS with tls.crt and tls.key from TLS_DIR; missing files are
// replaced by a self-signed certificate for the public_url host.
const configPath=process.env.AEZA_CONFIG||'/etc/aeza/config.toml';
const publicUrl=new URL(parseToml(readFileSync(configPath,'utf8')).server?.public_url??'');
const tlsDir=process.env.TLS_DIR||'/certs';

function tlsOptions(){
 const cert=path.join(tlsDir,'tls.crt'),key=path.join(tlsDir,'tls.key');
 if(!existsSync(cert)||!existsSync(key)){
  const host=publicUrl.hostname.replace(/^\[|\]$/g,'');
  const names=[...new Set([host,'localhost','127.0.0.1'])].map(name=>`${isIP(name)?'IP':'DNS'}:${name}`).join(',');
  mkdirSync(tlsDir,{recursive:true});
  execFileSync('openssl',['req','-x509','-newkey','rsa:2048','-nodes','-days','825','-subj',`/CN=${host}`,
   '-addext',`subjectAltName=${names}`,'-keyout',key,'-out',cert],{stdio:['ignore','ignore','inherit']});
  console.log(`Generated a self-signed certificate for ${names} in ${tlsDir}`);
 }
 return {cert:readFileSync(cert),key:readFileSync(key)};
}

const secure=publicUrl.protocol==='https:';
const app=secure?https.createServer(tlsOptions(),serve):http.createServer(serve);
app.on('upgrade',(req,socket,head)=>{
 if(new URL(req.url,'http://localhost').pathname==='/api/client/ws')return proxyUpgrade(req,socket,head,API);
 socket.destroy();
});

// With HTTPS the same port also answers plain HTTP, with a redirect to PUBLIC_URL:
// a TLS connection starts with a handshake record (0x16), HTTP with a method name.
const redirect=http.createServer(redirectToPublicUrl);
const listener=secure?net.createServer(socket=>{
 socket.on('error',()=>socket.destroy());
 socket.once('readable',()=>{
  const first=socket.read(1);
  if(!first)return void socket.destroy();
  socket.unshift(first);
  (first[0]===0x16?app:redirect).emit('connection',socket);
 });
}):app;
// The container port stays 80 in both modes; Compose maps FRONTEND_PORT to it.
listener.listen(80,'0.0.0.0',()=>console.log(`Frontend listening on port 80 (${secure?'HTTPS':'HTTP'}), public URL ${publicUrl.origin}`));
