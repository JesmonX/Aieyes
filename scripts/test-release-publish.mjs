import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtempSync, writeFileSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import publish from './publish-release.cjs';

async function fixture(run, {existing, failUpload=false}={}) {
  const directory=mkdtempSync(join(tmpdir(),'aieyes-publish-'));
  writeFileSync(join(directory,'installer.exe'),'fixture');
  writeFileSync(join(directory,'SHA256SUMS'),'checksums');
  let assets=existing ? [{id:9,name:'old.exe'}] : [], nextId=10;
  const calls=[];
  const repos={
    async getReleaseByTag(){if(existing)return {data:existing};throw Object.assign(new Error('not found'),{status:404});},
    async generateReleaseNotes(){return {data:{body:'Changes'}};},
    async createRelease(args){calls.push(['create',args]);return {data:{id:1,draft:true}};},
    async deleteReleaseAsset(args){calls.push(['delete',args]);assets=assets.filter(a=>a.id!==args.asset_id);},
    async uploadReleaseAsset(args){calls.push(['upload',args]);if(failUpload)throw new Error('upload interrupted');assets.push({id:nextId++,name:args.name,size:args.data.length,state:'uploaded'});},
    async updateRelease(args){calls.push(['publish',args]);},
    async listReleaseAssets(){return assets;},
  };
  const github={rest:{repos},paginate:async fn=>fn()};
  try {await run({invoke:()=>publish({github,context:{repo:{owner:'test',repo:'aieyes'}},tag:'v1.2.3',directory}),calls});}
  finally {rmSync(directory,{recursive:true,force:true});}
}

test('publication happens only after every attachment is uploaded',()=>fixture(async({invoke,calls})=>{
  await invoke();
  assert.deepEqual(calls.map(c=>c[0]),['create','upload','upload','publish']);
  assert.equal(calls[0][1].draft,true);
  assert.equal(calls.at(-1)[1].draft,false);
}));
test('public releases are immutable',()=>fixture(async({invoke,calls})=>{
  await assert.rejects(invoke(),/already public/);
  assert.deepEqual(calls,[]);
},{existing:{id:1,draft:false}}));
test('interrupted uploads retain a draft and reruns replace stale draft attachments',async()=>{
  await fixture(async({invoke,calls})=>{
    await assert.rejects(invoke(),/upload interrupted/);
    assert.equal(calls.some(c=>c[0]==='publish'),false);
  },{failUpload:true});
  await fixture(async({invoke,calls})=>{
    await invoke();
    assert.deepEqual(calls.map(c=>c[0]),['delete','upload','upload','publish']);
  },{existing:{id:1,draft:true}});
});
