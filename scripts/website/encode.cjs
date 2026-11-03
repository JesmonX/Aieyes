/* Encode renders, the website pet sheet and provider icons from existing artwork; nothing is generated. */
const fs=require('node:fs'),path=require('node:path'),{execFileSync}=require('node:child_process');
const {chromium}=require('../../apps/desktop/node_modules/playwright');
const sharp=require('../../apps/desktop/node_modules/sharp');
// The website plays idle, working and complete; the sheet keeps only those rows of the 8 × 9 original.
const PET_ROWS=[['idle',0],['working',1],['complete',4]],PET_CELL={width:192,height:208};
exports.PET_ROWS=PET_ROWS;
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
      // Render fixtures and promo scenes read the full 8 × 9 sheet from .local/website.
      fs.mkdirSync(work,{recursive:true});fs.writeFileSync(path.join(work,'pet-sheet.png'),await sharp(sheet).png().toBuffer());
      const {width,height}=await sharp(sheet).metadata();
      fs.writeFileSync(path.join(work,'pet-info.json'),JSON.stringify({width,height}));
      if(width!==PET_CELL.width*8||height!==PET_CELL.height*9)throw new Error(`Unexpected pet sheet ${width}×${height}`);
      const rows=await Promise.all(PET_ROWS.map(([,row])=>sharp(sheet).extract({left:0,top:row*PET_CELL.height,width,height:PET_CELL.height}).png().toBuffer()));
      const target=path.join(out,'pet-spritesheet.webp');
      await sharp({create:{width,height:PET_CELL.height*PET_ROWS.length,channels:4,background:{r:0,g:0,b:0,alpha:0}}})
        .composite(rows.map((input,index)=>({input,left:0,top:index*PET_CELL.height})))
        .webp({quality:85,alphaQuality:100,effort:6,smartSubsample:true}).toFile(target);
      console.log('Extracted original kiffin pet rows',PET_ROWS.map(([state,row])=>`${state}:${row}`).join(' '),fs.statSync(target).size,'bytes');
    }else if(process.argv.includes('--providers')){
      // Icons render at 28–32 CSS px; 64 px covers 2× screens. Sources are the app's own provider marks.
      const dir=path.join(out,'providers');
      for(const file of fs.readdirSync(dir).filter(name=>/^provider-.+\.png$/.test(name))){
        await sharp(path.join(dir,file)).resize(64,64,{fit:'contain',background:{r:0,g:0,b:0,alpha:0},kernel:'lanczos3'}).webp({quality:90,alphaQuality:100,effort:6}).toFile(path.join(dir,file.replace(/\.png$/,'.webp')));
      }
      console.log('Encoded provider icons at 64 px');
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
