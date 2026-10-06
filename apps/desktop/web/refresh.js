/* Shared keyboard and pointer behavior for all four refresh actions. */
function bindRefreshMenu(triggerID,menuID) {
  const trigger=document.querySelector('#'+triggerID),menu=document.querySelector('#'+menuID);if(!trigger||!menu)return;
  const close=(focus=false)=>{menu.hidden=true;trigger.setAttribute('aria-expanded','false');if(focus)trigger.focus();flushRender();};
  trigger.addEventListener('click',()=>{menu.hidden=!menu.hidden;trigger.setAttribute('aria-expanded',String(!menu.hidden));updateActivity();});
  const items=()=>[...menu.querySelectorAll('button:not(:disabled)')];
  trigger.addEventListener('keydown',e=>{if(['ArrowDown','ArrowUp'].includes(e.key)){e.preventDefault();menu.hidden=false;trigger.setAttribute('aria-expanded','true');items()[e.key==='ArrowDown'?0:items().length-1]?.focus();}});
  menu.addEventListener('keydown',e=>{const buttons=items(),i=buttons.indexOf(document.activeElement);const next=e.key==='ArrowDown'?(i+1)%buttons.length:e.key==='ArrowUp'?(i+buttons.length-1)%buttons.length:e.key==='Home'?0:e.key==='End'?buttons.length-1:null;if(next!=null){e.preventDefault();buttons[next]?.focus();}if(e.key==='Escape'){e.preventDefault();e.stopPropagation();close(true);}});
  menu.addEventListener('click',e=>{const button=e.target.closest('[data-refresh]');if(!button||button.disabled)return;const key=button.dataset.refresh;close(true);({scan,quotas,prices:syncPrices,hosts:sample})[key]();});
  document.addEventListener('pointerdown',e=>{if(!trigger.parentElement.contains(e.target))close();});
  document.addEventListener('keydown',e=>{if(e.key==='Escape'&&!menu.hidden){e.preventDefault();e.stopImmediatePropagation();close(true);}});
}
