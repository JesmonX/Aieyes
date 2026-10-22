const fs=require('node:fs'),path=require('node:path');
const {chromium}=require('../../apps/desktop/node_modules/playwright');
const root=path.resolve(__dirname,'../..'),publicDir=path.join(root,'apps/website/public');
(async()=>{
 const browser=await chromium.launch();try{
 const page=await browser.newPage({viewport:{width:1200,height:630},deviceScaleFactor:1});
 const font=fs.readFileSync(path.join(publicDir,'fonts/comic-neue-regular.woff2')).toString('base64');
 const image=(file,type)=>`data:image/${type};base64,${fs.readFileSync(path.join(publicDir,file)).toString('base64')}`;
 await page.setContent(`<!doctype html><style>@font-face{font-family:"Comic Neue";src:url(data:font/woff2;base64,${font}) format("woff2");font-weight:400}*{box-sizing:border-box}body{margin:0;width:1200px;height:630px;background:radial-gradient(ellipse at 70% 70%,#27213d,transparent 65%),#0b0d14;color:#eeeff6;font-family:"Comic Sans MS","Comic Neue",cursive;padding:65px;overflow:hidden}.brand{display:flex;align-items:center;gap:13px;font-size:29px;font-weight:600;letter-spacing:-1px}.brand img{width:48px;height:48px}h1{font-size:67px;font-weight:500;line-height:1.1;letter-spacing:-3px;margin:65px 0 27px;position:relative;z-index:2}h1 span{color:#bdb0ec}p{color:#9295ab;font-size:18px;line-height:1.8;margin:0}.app{position:absolute;left:655px;top:150px;width:770px;transform:rotate(-6deg);border-radius:12px;border:1px solid #c0a9ff40;box-shadow:0 25px 90px #0009}.url{position:absolute;bottom:55px;left:65px;color:#74788e;font-size:16px}</style><div class="brand"><img src="${image('brand.png','png')}">Aieyes</div><h1>Your AI agents.<br><span>One clear view.</span></h1><p>One Agent or many. Every account in view.<br>Switch accounts and APIs. Keep chatting.</p><img class="app" src="${image('screenshots/macos-overview.webp','webp')}"><div class="url">aieyes.2h3ngy4n.space · macOS / Windows / Linux</div>`);
 await page.evaluate(()=>document.fonts.ready);await page.waitForFunction(()=>[...document.images].every(image=>image.complete));await page.screenshot({path:path.join(publicDir,'og.png')});
 console.log('Generated social sharing image');
 }finally{await browser.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
