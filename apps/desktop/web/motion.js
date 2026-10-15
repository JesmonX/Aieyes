/* Finite Windows interactions. Rendering data does not itself imply navigation.
   CSS owns control/overlay transitions; this module owns measured view changes. */
(() => {
  const root=document.documentElement,reduced=matchMedia('(prefers-reduced-motion: reduce)'),contrast=matchMedia('(forced-colors: active)');
  const animations=new Map(),selections=new Map(),frames=new Set(),removals=new Map();
  const spring='cubic-bezier(.22,1.35,.36,1)';
  let lastView=null,navigation=null;
  const enabled=()=>root.dataset.platform==='windows'&&!reduced.matches&&!contrast.matches&&!document.hidden;
  function cancel(element) {
    const animation=animations.get(element);
    if(animation){animations.delete(element);animation.cancel();delete element.dataset.winAnimating;}
  }
  function run(element,keyframes,options={}) {
    if(!element)return null;
    cancel(element);
    if(!enabled()||!element.isConnected||!element.getClientRects().length)return null;
    const animation=element.animate(keyframes,{duration:240,easing:spring,...options});
    animations.set(element,animation);element.dataset.winAnimating=options.id??'interaction';
    const finish=()=>{if(animations.get(element)===animation){animations.delete(element);delete element.dataset.winAnimating;}animation.cancel();};
    animation.finished.then(finish,finish);return animation;
  }
  function enter(element,kind='page') {
    const distance=kind==='menu'?4:8;
    return run(element,[{opacity:0,transform:`translateY(${distance}px)`},{opacity:1,transform:'none'}],{duration:kind==='page'?220:240,id:kind});
  }
  const groups=['aside > nav','.settings-tabs','.settings-sidebar','.panel-tabs','.metric-picker'];
  function selected() {
    return groups.flatMap(selector=>[...document.querySelectorAll(selector)].map((group,i)=>{
      const button=group.querySelector(':scope > button.active,:scope > button[aria-pressed=true]');
      if(!button)return null;
      return {key:selector+':'+i,button,value:button.id||button.getAttribute('data-page')||button.textContent.trim(),rect:button.getBoundingClientRect()};
    }).filter(Boolean));
  }
  function captureNavigation() {
    if(enabled()&&!navigation)navigation=selected();
  }
  function clearSelection(button) {
    clearTimeout(selections.get(button));selections.delete(button);button.removeAttribute('data-win-selection');
    for(const name of ['x','y','sx','sy'])button.style.removeProperty('--win-selection-'+name);
  }
  function didRender(key,target=document.querySelector('#content')) {
    const before=navigation;navigation=null;
    const changed=lastView!==key,initial=lastView===null;lastView=key;
    if(!enabled())return;
    if(before)for(const current of selected()){
      const previous=before.find(item=>item.key===current.key);
      if(!previous||previous.value===current.value||!previous.rect.width||!current.rect.width)continue;
      clearSelection(current.button);
      const a=previous.rect,b=current.rect;
      current.button.style.setProperty('--win-selection-x',(a.x+a.width/2-b.x-b.width/2)+'px');
      current.button.style.setProperty('--win-selection-y',(a.y+a.height/2-b.y-b.height/2)+'px');
      current.button.style.setProperty('--win-selection-sx',a.width/b.width);
      current.button.style.setProperty('--win-selection-sy',a.height/b.height);
      current.button.setAttribute('data-win-selection','');
      selections.set(current.button,setTimeout(()=>clearSelection(current.button),280));
    }
    // The native panel's open event supplies its initial entrance separately.
    if(changed&&!(initial&&root.dataset.surface==='panel'))enter(target,'page');
  }
  function nextFrame(callback) {
    const id=requestAnimationFrame(()=>{frames.delete(id);if(enabled())callback();});frames.add(id);return id;
  }
  function sidebar(change) {
    if(!enabled()){change();return;}
    const main=document.querySelector('main'),aside=document.querySelector('aside');
    cancel(main);cancel(aside);
    const before=main.getBoundingClientRect(),width=aside.getBoundingClientRect().width;
    change();
    const after=main.getBoundingClientRect(),next=aside.getBoundingClientRect().width;
    // Commit layout once. Translate content and reveal the sidebar without
    // animating grid widths or redrawing the charts for every animation frame.
    run(main,[{transform:`translateX(${before.x-after.x}px)`,opacity:.75},{transform:'none',opacity:1}],{duration:260,id:'sidebar'});
    if(next>width)run(aside,[{clipPath:`inset(0 ${next-width}px 0 0)`},{clipPath:'inset(0 0 0 0)'}],{duration:260,id:'sidebar'});
    else enter(aside,'sidebar');
    navigation=null;
  }
  function panel(open) {
    if(open)enter(document.querySelector('.panel-body'),'panel');else cancelAll();
  }
  function finishMenuExits() {
    if(root.dataset.platform!=='windows')return;
    for(const menu of document.querySelectorAll('.multi-panel:not(:popover-open),.win-select-menu:not(:popover-open)'))for(const animation of menu.getAnimations())animation.finish();
  }
  function removeAfterExit(element) {
    if(!element)return;
    clearTimeout(removals.get(element));
    if(!enabled()){element.remove();return;}
    removals.set(element,setTimeout(()=>{removals.delete(element);if(!element.open)element.remove();},150));
  }
  function cancelAll() {
    for(const element of [...animations.keys()])cancel(element);
    for(const element of [...selections.keys()])clearSelection(element);
    for(const id of frames)cancelAnimationFrame(id);frames.clear();navigation=null;
    for(const [element,timer] of removals){clearTimeout(timer);if(!element.open)element.remove();}removals.clear();
  }
  document.addEventListener('click',event=>{
    if(!enabled())return;
    const summary=event.target.closest('summary');
    if(!summary||event.target.closest('button,a,input,select,textarea')||summary.parentElement.dataset.collapsible==='false')return;
    const details=summary.parentElement;if(details.matches('#panel-more,.machine-menu'))return;
    const wasOpen=details.open,container=details.closest('.quota-card,.card')??details;
    const siblings=[...container.parentElement.children].filter(el=>el!==container).map(el=>({el,rect:el.getBoundingClientRect()})).filter(({rect})=>rect.bottom>=0&&rect.top<=innerHeight);
    nextFrame(()=>nextFrame(()=>{
      if(!details.isConnected||details.open===wasOpen)return;
      for(const {el,rect} of siblings){
        if(!el.isConnected)continue;
        const next=el.getBoundingClientRect(),delta=rect.top-next.top;
        if(Math.abs(delta)>.5)run(el,[{transform:`translateY(${delta}px)`},{transform:'none'}],{duration:220,id:'disclosure'});
      }
      if(details.open){
        const content=details.matches('.quota-disclosure')?details.nextElementSibling:details.querySelector(':scope > :not(summary)');
        enter(content,'disclosure');
      }
    }));
  },true);
  reduced.addEventListener('change',cancelAll);contrast.addEventListener('change',cancelAll);
  document.addEventListener('visibilitychange',()=>{if(document.hidden)cancelAll();});
  window.addEventListener('pagehide',cancelAll);
  window.AieyesMotion={captureNavigation,didRender,enter,cancel,cancelAll,sidebar,panel,finishMenuExits,removeAfterExit};
})();
