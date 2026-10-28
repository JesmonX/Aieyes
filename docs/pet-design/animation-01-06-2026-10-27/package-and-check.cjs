const fs=require('fs'),path=require('path'),os=require('os'),assert=require('assert'),{spawnSync}=require('child_process');
const dir=__dirname,repo=path.resolve(dir,'../../..'),root=fs.mkdtempSync(path.join(os.tmpdir(),'aieyes-pet-01-06-')),reports=[];
for(const id of ['01-folded-ear-scout','06-inkdrop-artisan']){
 const zip=spawnSync('/usr/bin/zip',['-j','-q',path.join(dir,id+'.codex-pet.zip'),'pet.json','spritesheet.png'],{cwd:path.join(dir,id),encoding:'utf8'});if(zip.status!==0)throw Error(zip.stderr);
 const result=spawnSync(path.join(repo,'target/debug/aieyes-core'),['--data-dir',root,'--call','pets.import',JSON.stringify({input:path.join(dir,id+'.codex-pet.zip')})],{env:{...process.env,AIEYES_OFFLINE:'1'},encoding:'utf8',timeout:30000});if(result.status!==0)throw Error(result.stderr);
 const value=JSON.parse(result.stdout);assert.equal(value.error,null);assert.equal(value.pet.rows,9);assert.equal(value.pet.columns,8);assert.equal(value.pet.cellWidth,192);assert.equal(value.pet.cellHeight,208);
 reports.push({id,result:value});console.log(id+': imported successfully; 8x9 cells of 192x208.');
}
fs.writeFileSync(path.join(dir,'import-check.json'),JSON.stringify({isolatedDataDirectory:root,binary:'target/debug/aieyes-core',checkedAt:new Date().toISOString(),reports},null,2)+'\n');
