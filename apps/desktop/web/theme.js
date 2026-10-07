/* Saved application appearance, shared by every web surface. */
(() => {
  const media = matchMedia('(prefers-color-scheme: dark)');
  let current = {theme:'system',accent:'indigo'}, nativeKey = '';
  try { current = JSON.parse(localStorage.getItem('aieyes.appearance') || 'null') || current; } catch (_) {}
  function apply(value = current) {
    current = {theme:['system','light','dark'].includes(value?.theme)?value.theme:'system',accent:['indigo','blue','teal','purple'].includes(value?.accent)?value.accent:'indigo'};
    const root=document.documentElement;
    const previous=root.dataset.theme+root.dataset.accent;
    root.dataset.theme=current.theme==='system'?(media.matches?'dark':'light'):current.theme;
    root.dataset.accent=current.accent;
    if(previous!==root.dataset.theme+root.dataset.accent)window.dispatchEvent(new Event('aieyes:appearance')); 
    try { localStorage.setItem('aieyes.appearance',JSON.stringify(current)); } catch (_) {}
    const key=JSON.stringify(current);
    if(window.__TAURI__?.core&&nativeKey!==key){nativeKey=key;window.__TAURI__.core.invoke('desktop_appearance',{theme:current.theme,accent:current.accent}).catch(()=>{nativeKey='';});}
  }
  media.addEventListener('change',()=>apply(current));
  window.addEventListener('storage',e=>{if(e.key==='aieyes.appearance'){try{apply(JSON.parse(e.newValue));}catch(_){}}});
  window.AieyesTheme={apply};
  apply();
})();
