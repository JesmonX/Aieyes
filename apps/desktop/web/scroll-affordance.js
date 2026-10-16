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
