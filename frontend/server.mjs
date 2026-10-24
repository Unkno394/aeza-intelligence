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

function serve(req,res){
 const pathname=new URL(req.url,'http://localhost').pathname;
 if(pathname.startsWith('/api/market/v1/')||pathname.startsWith('/api/market/v2/'))return proxy(req,res,'http://simulator:8091');
 if(pathname.startsWith('/api/'))return proxy(req,res,'http://host.docker.internal:3000');
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

http.createServer(serve).listen(80,'0.0.0.0',()=>console.log('Frontend listening on port 80'));
