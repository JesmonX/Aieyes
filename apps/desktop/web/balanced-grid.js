// Only observe summary grids; detailed device lists retain their virtualization.
(() => {
  const pending=new Set(), mounted=new WeakSet();let frame=0;
  const schedule=element=>{pending.add(element);if(!frame)frame=requestAnimationFrame(()=>{
    frame=0;
    for(const grid of pending){
      if(!grid.isConnected)continue;
      const count=grid.children.length, width=grid.clientWidth;
      const style=getComputedStyle(grid), gap=parseFloat(style.columnGap)||10;
      const columns=AieyesUI.balancedColumns(width,count,290,gap);
      grid.style.setProperty('--balanced-columns',columns);
      grid.dataset.singleLast=String(count>columns&&count%columns===1);
    }
    pending.clear();
  });};
  const resize=new ResizeObserver(rows=>rows.forEach(row=>schedule(row.target)));
  const discover=node=>{
    if(node.nodeType!==1)return;
    const grids=[...(node.matches('.balanced-grid')?[node]:[]),...node.querySelectorAll('.balanced-grid')];
    for(const grid of grids){if(!mounted.has(grid)){mounted.add(grid);resize.observe(grid);}schedule(grid);}
  };
  const mutations=new MutationObserver(rows=>{for(const row of rows){
    const grid=row.target.closest?.('.balanced-grid');if(grid)schedule(grid);
    row.addedNodes.forEach(discover);
    row.removedNodes.forEach(node=>{if(node.nodeType===1)for(const grid of [node,...node.querySelectorAll('.balanced-grid')])if(grid.matches('.balanced-grid')&&!grid.isConnected){resize.unobserve(grid);mounted.delete(grid);}});
  }});
  const start=()=>{discover(document.documentElement);mutations.observe(document.documentElement,{childList:true,subtree:true});};
  if(document.readyState==='loading')document.addEventListener('DOMContentLoaded',start,{once:true});else start();
})();
