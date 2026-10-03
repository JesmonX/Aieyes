import test from 'node:test';
import assert from 'node:assert/strict';
import '../apps/desktop/web/multiselect.js';

const S=globalThis.AieyesSelect;
test('default all includes new devices; clear persists an explicit empty group',()=>{
  const ids=['eth0','eth1'];
  assert.deepEqual(S.deviceSelected([], 'network', ids),ids);
  const cleared=S.writeDevices(['gpu:0'],'network',[]);
  assert.deepEqual(cleared,['gpu:0','network:__none__']);
  assert.deepEqual(S.deviceSelected(S.parse(cleared.join(', ')),'network',ids),[]);
  const all=S.writeDevices(cleared,'network',ids,true);
  assert.deepEqual(all,['gpu:0']);
  assert.deepEqual(S.deviceSelected(all,'network',[...ids,'eth2']),[...ids,'eth2']);
});
test('filtered batch edits preserve hidden and unavailable choices',()=>{
  const options=[{id:'eth0',label:'网卡'}, {id:'eth1',label:'网卡'}, {id:'ib0',label:'高速网络'}];
  const targets=S.filterOptions(options,'ETH').map(o=>o.id);
  const selected=['eth0','ib0','old0'];
  const inverted=S.applySelection(selected,targets,'invert');
  assert.deepEqual(new Set(inverted),new Set(['eth1','ib0','old0']));
  assert.deepEqual(new Set(S.applySelection(inverted,targets,'invert')),new Set(selected));
  assert.deepEqual(S.applySelection(selected,targets,'clear'),['ib0','old0']);
  assert.deepEqual(S.filterOptions(options,'高速').map(o=>o.id),['ib0']);
  assert.deepEqual(S.applySelection(selected,[],'invert'),selected);
});
test('device groups are isolated; aggregate CPU is never a selectable core',()=>{
  const tokens=['cpu:cpu','cpu:cpu0','cpu:__none__','network:eth0','gpu:2'];
  assert.deepEqual(S.deviceIds(tokens,'cpu'),['cpu0']);
  const next=S.writeDevices(tokens,'cpu',['cpu','cpu1','cpu1']);
  assert.deepEqual(next,['network:eth0','gpu:2','cpu:cpu1']);
  assert.deepEqual(S.parse('network:eth0, , gpu:2, network:eth0'),['network:eth0','gpu:2']);
});
test('hundreds of devices retain explicit subsets when discovery changes',()=>{
  const ids=Array.from({length:512},(_,i)=>`cpu${i}`);
  const selected=S.applySelection(ids,ids.filter((_,i)=>i%2===0),'invert');
  assert.equal(selected.length,256);
  const saved=S.writeDevices([],'cpu',selected);
  assert.deepEqual(S.deviceSelected(saved,'cpu',[...ids,'cpu512']),selected);
});
