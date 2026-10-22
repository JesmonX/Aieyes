/* Encode renders and extract existing pet frames; no image-generation dependencies. */
const fs=require('node:fs'),path=require('node:path'),{execFileSync}=require('node:child_process');
const {chromium}=require('../../apps/desktop/node_modules/playwright');
const root=path.resolve(__dirname,'../..'),work=path.join(root,'.local/website'),out=path.join(root,'apps/website/public');
async function encode(page,buffer,{type='image/webp',row=null}={}){
  const url='data:image/png;base64,'+buffer.toString('base64');
  return Buffer.from(await page.evaluate(async({url,type,row})=>{
    const image=new Image();image.src=url;await image.decode();
    const canvas=document.createElement('canvas');
    canvas.width=row===null?image.width:image.width/8;canvas.height=row===null?image.height:image.height/9;
    const context=canvas.getContext('2d');
    if(row===null)context.drawImage(image,0,0);else context.drawImage(image,0,row*canvas.height,canvas.width,canvas.height,0,0,canvas.width,canvas.height);
    return canvas.toDataURL(type,.9).split(',')[1];
  },{url,type,row}),'base64');
}
exports.encode=encode;
if(require.main===module)(async()=>{
  const browser=await chromium.launch();
  try{
    const page=await browser.newPage();
    if(process.argv.includes('--pet')){
      const sheet=execFileSync('python3',['-c','from zipfile import ZipFile; import sys; sys.stdout.buffer.write(ZipFile(sys.argv[1]).read("spritesheet.webp"))',path.join(root,'crates/core/assets/yuexinmiao1.codex-pet.zip')]);
      fs.writeFileSync(path.join(out,'pet-spritesheet.webp'),sheet);
      const png=await encode(page,sheet,{type:'image/png'});fs.writeFileSync(path.join(work,'pet-sheet.png'),png);
      const dimensions=await page.evaluate(async url=>{const image=new Image();image.src=url;await image.decode();return{width:image.width,height:image.height};},'data:image/png;base64,'+png.toString('base64'));
      fs.writeFileSync(path.join(work,'pet-info.json'),JSON.stringify(dimensions));
      for(const [state,row] of [['idle',0],['working',1],['complete',4]])fs.writeFileSync(path.join(out,`pet-${state}.webp`),await encode(page,png,{row}));
      console.log('Extracted original kiffin pet artwork');
    }else{
      const language=process.argv[process.argv.indexOf('--language')+1]||'all';
      for(const locale of language==='all'||!['en','zh-CN'].includes(language)?['en','zh-CN']:[language]){
        const input=path.join(work,'native',locale),target=path.join(out,'screenshots',locale);fs.mkdirSync(target,{recursive:true});
        for(const file of fs.readdirSync(input).filter(name=>/^macos-(overview|analytics|servers|panel)\.png$/.test(name))){
          const bytes=await encode(page,fs.readFileSync(path.join(input,file))),name=file.replace('.png','.webp');
          fs.writeFileSync(path.join(target,name),bytes);
          if(locale==='en')fs.writeFileSync(path.join(out,'screenshots',name),bytes);
          console.log('Encoded',locale,file);
        }
      }
    }
  }finally{await browser.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
