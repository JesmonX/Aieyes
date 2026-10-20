/* Each surface owns its filters. Legacy preferences are copied, never removed. */
(() => {
  const keys=['provider','accountKey','sourceId','model','days','dateRange','cost'];
  const defaults=()=>({provider:'',accountKey:'',sourceId:'',model:'',days:1,dateRange:null,cost:false});
  function normalize(value={},scope) {
    const next=defaults();
    for(const key of keys.slice(0,4))if(typeof value[key]==='string')next[key]=value[key];
    if(next.provider==='agy')next.provider='antigravity';
    // Old saved account filters must not silently hide shared Agent usage.
    next.accountKey='';
    if([1,7,30,90,365].includes(value.days))next.days=value.days;
    if(['all','custom'].includes(value.dateRange?.kind))next.dateRange=structuredClone(value.dateRange);
    next.cost=value.cost===true;
    if(scope==='overview'||scope==='panel'){next.days=1;next.dateRange=null;next.sourceId='';next.model='';}
    return next;
  }
  function create(storage) {
    const read=key=>{try{return JSON.parse(storage.getItem(key)||'null');}catch(_){return null;}};
    const write=(key,value)=>{try{storage.setItem(key,JSON.stringify(value));}catch(_){}};
    const prefix='aieyes.view.v3.';
    return {
      read(scope){return normalize(read(prefix+scope)??read(scope==='panel'?'aieyes.panel.view':'aieyes.dashboard.filters.v2')??{},scope);},
      save(scope,value){write(prefix+scope,normalize(value,scope));},
      position(key){return read(prefix+'position.'+key)??{window:[0,0],panel:0,opened:[],closed:[],scrollers:[]};},
      savePosition(key,value){write(prefix+'position.'+key,{...value,focus:null});},
      normalize
    };
  }
  globalThis.AieyesViewState={create,normalize,keys};
})();
