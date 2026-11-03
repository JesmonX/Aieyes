/* Social sharing images per language: real product screenshot, site fonts and the original pet frame. */
const fs=require('node:fs'),path=require('node:path'),{pathToFileURL}=require('node:url');
const {chromium}=require('../../apps/desktop/node_modules/playwright');
const root=path.resolve(__dirname,'../..'),publicDir=path.join(root,'apps/website/public');
const OG={en:{file:'og.jpg',lines:'One Agent or many. Every account in view.<br>Switch accounts and APIs. Keep chatting.'},'zh-CN':{file:'og-zh-CN.jpg',lines:'单 Agent 或多 Agent，每个账户都在眼前。<br>切换账户和 API，接着对话。'}};
exports.OG=OG;
const data=(file,type)=>`data:${type};base64,${fs.readFileSync(path.join(publicDir,file)).toString('base64')}`;
const escape=value=>String(value).replaceAll('&','&amp;').replaceAll('<','&lt;');
if(require.main===module)(async()=>{
 const {en,zh}=await import(pathToFileURL(path.join(root,'apps/website/content.mjs')).href);
 const faces=JSON.parse(fs.readFileSync(path.join(publicDir,'fonts/sources.json'),'utf8')).map(face=>`@font-face{font-family:"${face.family}";src:url(${data('fonts/'+face.file,'font/woff2')}) format("woff2");font-weight:${face.weight}${face.unicodeRange?';unicode-range:'+face.unicodeRange:''}}`).join('');
 const browser=await chromium.launch();try{
 const page=await browser.newPage({viewport:{width:1200,height:630},deviceScaleFactor:1});
 for(const c of [en,zh]){
  const {file,lines}=OG[c.lang];
  await page.setContent(`<!doctype html><html lang="${c.lang}"><style>${faces}*{box-sizing:border-box}body{margin:0;width:1200px;height:630px;background:radial-gradient(ellipse at 70% 70%,#27213d,transparent 65%),#0b0d14;color:#eeeff6;font-family:"Comic Neue","LXGW WenKai",cursive;padding:65px;overflow:hidden}.brand{display:flex;align-items:center;gap:13px;font-size:29px;font-weight:700;letter-spacing:-.5px}.brand img{width:48px;height:48px}h1{font-size:67px;font-weight:700;line-height:1.1;letter-spacing:-2px;margin:65px 0 27px;position:relative;z-index:2}:lang(zh-CN) h1{font-weight:400;letter-spacing:0}h1 span{color:#bdb0ec}p{color:#9295ab;font-size:20px;line-height:1.8;margin:0}.app{position:absolute;left:655px;top:150px;width:770px;transform:rotate(-6deg);border-radius:12px;border:1px solid #c0a9ff40;box-shadow:0 25px 90px #0009}.pet{position:absolute;left:560px;top:418px;width:144px;height:156px;background:url(${data('pet-spritesheet.webp','image/webp')}) 0 0/1152px 468px no-repeat;filter:drop-shadow(0 14px 12px #0007);z-index:3}.url{position:absolute;bottom:55px;left:65px;color:#74788e;font-size:18px}</style><div class="brand"><img src="${data('brand.png','image/png')}">Aieyes</div><h1>${escape(c.hero[0])}<br><span>${escape(c.hero[1])}</span></h1><p>${lines}</p><img class="app" src="${data(`screenshots/${c.lang}/macos-overview.webp`,'image/webp')}"><div class="pet"></div><div class="url">aieyes.2h3ngy4n.space · macOS / Windows / Linux</div></html>`);
  await page.evaluate(()=>document.fonts.ready);await page.waitForFunction(()=>[...document.images].every(image=>image.complete));
  await page.screenshot({path:path.join(publicDir,file),type:'jpeg',quality:82});
  console.log('Generated social sharing image',file,fs.statSync(path.join(publicDir,file)).size,'bytes');
 }
 }finally{await browser.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
