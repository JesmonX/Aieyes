const fs=require('fs'),path=require('path'),assert=require('assert');
const sharp=require('../../../apps/desktop/node_modules/sharp');
const root=__dirname,report=JSON.parse(fs.readFileSync(path.join(root,'assembly-report.json')));
(async()=>{for(const pet of report.characters){
 const dir=path.join(root,pet.id),anim=path.join(dir,'animations');fs.mkdirSync(anim,{recursive:true});const all=[],allDelay=[];
 for(const row of pet.rows){const raw=await Promise.all(row.items.map(f=>sharp(path.join(dir,f.file)).raw().toBuffer()));
 const filename=path.join(anim,String(row.row).padStart(2,'0')+'-'+row.action+'.webp');
 await sharp(Buffer.concat(raw),{raw:{width:192,height:208*raw.length,channels:4,pageHeight:208}}).webp({lossless:true,loop:0,delay:row.frameDurationsMs}).toFile(filename);
 const meta=await sharp(filename,{animated:true}).metadata();assert.equal(meta.pages,row.frames);assert.deepEqual(meta.delay,row.frameDurationsMs);
 for(let cycle=0;cycle<3;cycle++){all.push(...raw);allDelay.push(...row.frameDurationsMs);}
 }
 await sharp(Buffer.concat(all),{raw:{width:192,height:208*all.length,channels:4,pageHeight:208}}).webp({lossless:true,loop:0,delay:allDelay}).toFile(path.join(dir,'all-actions.webp'));
 console.log(pet.id+': 9 lossless animated WebP loops; encoded frame counts and exact delays verified.');
}})().catch(e=>{console.error(e);process.exit(1)});
