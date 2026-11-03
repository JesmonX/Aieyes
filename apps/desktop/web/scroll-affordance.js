/* Reveal thumbs without changing scrollbar width or content geometry. */
(() => {
  const timers=new WeakMap();
  document.addEventListener('scroll',event=>{
    const target=event.target===document?document.documentElement:event.target;
    if(!(target instanceof Element))return;
    target.dataset.scrolling='true';clearTimeout(timers.get(target));
    timers.set(target,setTimeout(()=>{delete target.dataset.scrolling;timers.delete(target);},800));
  },{capture:true,passive:true});
})();
/* Single-line tab strips (.tab-scroller): edges fade only on the side that hides tabs, a vertical
   wheel scrolls sideways, and the selected tab is kept in view after every render. */
(() => {
  const SCROLLER='.tab-scroller',FADE=24,mounted=new WeakSet();
  const update=el=>{
    const max=el.scrollWidth-el.clientWidth,value=[el.scrollLeft>1&&'start',el.scrollLeft<max-1&&'end'].filter(Boolean).join(' ');
    if((el.dataset.overflow??'')!==value){if(value)el.dataset.overflow=value;else delete el.dataset.overflow;}
  };
  const reveal=el=>{
    const active=el.querySelector(':scope > :is([aria-pressed=true],[aria-current],.active)');if(!active||el.scrollWidth<=el.clientWidth)return;
    const start=active.offsetLeft,end=start+active.offsetWidth;
    if(start<el.scrollLeft+FADE)el.scrollLeft=start-FADE;else if(end>el.scrollLeft+el.clientWidth-FADE)el.scrollLeft=end-el.clientWidth+FADE;
  };
  const sizes=new ResizeObserver(rows=>{for(const {target} of rows){if(target.isConnected)update(target);else{sizes.unobserve(target);mounted.delete(target);}}});
  const mount=node=>{
    if(node.nodeType!==1)return;
    for(const el of [...(node.matches(SCROLLER)?[node]:[]),...node.querySelectorAll(SCROLLER)]){if(!mounted.has(el)){mounted.add(el);sizes.observe(el);}reveal(el);update(el);}
  };
  new MutationObserver(rows=>{for(const row of rows)row.addedNodes.forEach(mount);}).observe(document.documentElement,{childList:true,subtree:true});
  document.addEventListener('scroll',event=>{if(event.target instanceof Element&&event.target.matches(SCROLLER))update(event.target);},{capture:true,passive:true});
  document.addEventListener('wheel',event=>{
    const el=event.target instanceof Element?event.target.closest(SCROLLER):null;
    if(!el||event.ctrlKey||Math.abs(event.deltaX)>=Math.abs(event.deltaY))return;
    const max=el.scrollWidth-el.clientWidth,next=Math.max(0,Math.min(max,el.scrollLeft+event.deltaY*(event.deltaMode===1?16:1)));
    if(max<=0||next===el.scrollLeft)return;
    event.preventDefault();el.scrollLeft=next;
  },{passive:false});
  addEventListener('aieyes:language',()=>document.querySelectorAll(SCROLLER).forEach(update));
})();
