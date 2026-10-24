import {useEffect,useRef} from 'react';

const vertexSource=`#version 300 es
void main(){vec2 p=vec2(float((gl_VertexID<<1)&2),float(gl_VertexID&2));gl_Position=vec4(p*2.0-1.0,0.0,1.0);}`;

const fragmentSource=`#version 300 es
precision highp float;
uniform vec2 uResolution;
uniform float uTime;
uniform float uDpr;
out vec4 fragColor;
float roundedBox(vec2 p,vec2 halfSize,float radius){vec2 q=abs(p)-halfSize+radius;return length(max(q,0.0))+min(max(q.x,q.y),0.0)-radius;}
void main(){
 vec2 uv=gl_FragCoord.xy/uResolution;
 vec2 pitch=vec2(16.0,28.0)*uDpr;
 vec2 cell=mod(gl_FragCoord.xy,pitch);
 vec2 local=cell/pitch;
 vec2 p=cell-pitch*0.5;
 vec2 halfSize=(pitch-vec2(3.5*uDpr))*0.5;
 float sdf=roundedBox(p,halfSize,2.0*uDpr);
 float mask=1.0-smoothstep(-0.8*uDpr,0.8*uDpr,sdf);
 float phase=uv.x*8.5+uv.y*5.0+sin(uv.x*4.0-uTime*0.12)*0.55;
 float swell=0.5+0.5*sin(phase-uTime*0.2);
 float crest=pow(max(0.0,sin(phase*1.3-uTime*0.28)),7.0);
 float edge=1.0-smoothstep(0.0,0.16,abs(local.y-0.5));
 vec3 deep=vec3(0.018,0.085,0.17);
 vec3 blue=vec3(0.035,0.27,0.5);
 vec3 lit=vec3(0.12,0.52,0.84);
 vec3 color=mix(deep,blue,swell*0.65+0.1);
 color=mix(color,lit,crest*0.48+edge*0.08);
 float alpha=mask*(0.24+swell*0.12+crest*0.18);
 fragColor=vec4(color*alpha,alpha);
}`;

function makeShader(gl:WebGL2RenderingContext,type:number,source:string){
 const shader=gl.createShader(type);if(!shader)return null;gl.shaderSource(shader,source);gl.compileShader(shader);
 if(!gl.getShaderParameter(shader,gl.COMPILE_STATUS)){gl.deleteShader(shader);return null;}return shader;
}

export default function BlueSlatsBackground(){
 const canvasRef=useRef<HTMLCanvasElement>(null);
 useEffect(()=>{
  const canvas=canvasRef.current;if(!canvas)return;
  const gl=canvas.getContext('webgl2',{alpha:true,premultipliedAlpha:true,antialias:false,depth:false,stencil:false});
  if(!gl)return;
  const vertex=makeShader(gl,gl.VERTEX_SHADER,vertexSource);const fragment=makeShader(gl,gl.FRAGMENT_SHADER,fragmentSource);
  if(!vertex||!fragment){if(vertex)gl.deleteShader(vertex);if(fragment)gl.deleteShader(fragment);return;}
  const program=gl.createProgram();if(!program){gl.deleteShader(vertex);gl.deleteShader(fragment);return;}
  gl.attachShader(program,vertex);gl.attachShader(program,fragment);gl.linkProgram(program);
  if(!gl.getProgramParameter(program,gl.LINK_STATUS)){gl.deleteProgram(program);gl.deleteShader(vertex);gl.deleteShader(fragment);return;}
  const resolution=gl.getUniformLocation(program,'uResolution');const time=gl.getUniformLocation(program,'uTime');const dprLocation=gl.getUniformLocation(program,'uDpr');
  const reduceMotion=window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  let raf=0,lastRender=0,lastTime=0,elapsed=0,alive=true,visible=true,dpr=1;
  const resize=()=>{
   const width=Math.max(1,canvas.clientWidth);const height=Math.max(1,canvas.clientHeight);
   dpr=Math.min(window.devicePixelRatio||1,1.1,Math.sqrt(3_000_000/(width*height)));
   const nextWidth=Math.round(width*dpr),nextHeight=Math.round(height*dpr);
   if(canvas.width!==nextWidth)canvas.width=nextWidth;if(canvas.height!==nextHeight)canvas.height=nextHeight;
   start();
  };
  const draw=(now:number)=>{
   raf=0;if(!alive||!visible||document.hidden)return;
   if(lastRender&&now-lastRender<1000/24){raf=requestAnimationFrame(draw);return;}
   const dt=lastTime?Math.min((now-lastTime)/1000,.1):0;lastTime=now;lastRender=now;
   if(!reduceMotion)elapsed+=dt;
   gl.viewport(0,0,canvas.width,canvas.height);gl.clearColor(0,0,0,0);gl.clear(gl.COLOR_BUFFER_BIT);
   gl.useProgram(program);gl.uniform2f(resolution,canvas.width,canvas.height);gl.uniform1f(time,elapsed);gl.uniform1f(dprLocation,dpr);gl.drawArrays(gl.TRIANGLES,0,3);
   if(!reduceMotion)raf=requestAnimationFrame(draw);
  };
  function start(){if(!raf&&alive&&visible&&!document.hidden)raf=requestAnimationFrame(draw);}
  const onVisibility=()=>{if(document.hidden){cancelAnimationFrame(raf);raf=0;lastTime=0;}else start();};
  const onContextLost=(event:Event)=>{event.preventDefault();alive=false;cancelAnimationFrame(raf);raf=0;};
  canvas.addEventListener('webglcontextlost',onContextLost);document.addEventListener('visibilitychange',onVisibility);
  const resizeObserver=new ResizeObserver(resize);resizeObserver.observe(canvas);
  const intersectionObserver=new IntersectionObserver(([entry])=>{visible=entry.isIntersecting;if(visible)start();else{cancelAnimationFrame(raf);raf=0;lastTime=0;}});
  intersectionObserver.observe(canvas);resize();
  return()=>{alive=false;cancelAnimationFrame(raf);resizeObserver.disconnect();intersectionObserver.disconnect();document.removeEventListener('visibilitychange',onVisibility);canvas.removeEventListener('webglcontextlost',onContextLost);gl.deleteProgram(program);gl.deleteShader(vertex);gl.deleteShader(fragment);};
 },[]);
 return <canvas ref={canvasRef} className="blue-slats-background" aria-hidden="true"/>;
}
