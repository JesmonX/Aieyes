/* Timing is generated from core-contracts.json; both pet and library use this clock. */
window.AieyesPetAnimation=(()=>{
  let uniform = false;
  function durations(state){
    const count=Math.max(1,Math.min(8,Math.trunc(Number(state.frames))||1));
    if(uniform)return Array(count).fill(100);
    const supplied=state.frameDurationsMs;
    const values=supplied==null||Array.isArray(supplied)&&!supplied.length?window.AieyesContract.petFrameDurations?.[state.row]:supplied;
    if(Array.isArray(values)&&values.length===count&&values.every(ms=>Number.isFinite(ms)&&ms>0)&&Number.isFinite(values.reduce((sum,ms)=>sum+ms,0)))return values;
    return Array(count).fill(Number.isFinite(state.frameMs)&&state.frameMs>0&&Number.isFinite(state.frameMs*count)?state.frameMs:100);
  }
  function frame(state,elapsed,still=false){
    const row=Math.max(0,Math.min(8,Number(state.row)||0));
    if(still)return row*8;
    const timing=durations(state),cycle=timing.reduce((n,ms)=>n+ms,0);
    let time=Math.max(0,Number(elapsed)||0)%cycle;
    for(let index=0;index<timing.length;index++){if(time<timing[index])return row*8+index;time-=timing[index];}
    return row*8;
  }
  function idle(){const timing=window.AieyesContract.petFrameDurations?.[0];return {row:0,frames:timing?.length??6,frameDurationsMs:timing,frameMs:100};}
  return {setUniform:value=>{uniform=!!value;},durations,frame,idle,signature:state=>[state.row,...durations(state)].join(':')};
})();
