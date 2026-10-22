const platformButtons=[...document.querySelectorAll('[data-platform]')];
let platform='macos';
try{if(sessionStorage.getItem('aieyes.previewPlatform')==='windows')platform='windows';}catch{}
function setPlatform(value){
  platform=value;document.documentElement.dataset.platform=value;
  platformButtons.forEach(button=>button.setAttribute('aria-pressed',String(button.dataset.platform===value)));
  document.querySelectorAll('img[data-preview]').forEach(image=>{
    image.src=`/screenshots/${value}-${image.dataset.preview}.webp`;
    image.alt=image.alt.replace(/ — (?:macOS|Windows)$/,'')+` — ${value==='macos'?'macOS':'Windows'}`;
  });
  try{sessionStorage.setItem('aieyes.previewPlatform',value);}catch{}
}
platformButtons.forEach(button=>button.addEventListener('click',()=>setPlatform(button.dataset.platform)));
setPlatform(platform);
const featureTabs=[...document.querySelectorAll('[data-feature-tab]')];
const featurePanels=[...document.querySelectorAll('[data-feature-panel]')];
function setFeature(value,updateHash=true){
  const id=featureTabs.some(button=>button.dataset.featureTab===value)?value:'overview';
  featureTabs.forEach(button=>{const active=button.dataset.featureTab===id;button.setAttribute('aria-selected',String(active));button.tabIndex=active?0:-1;});
  featurePanels.forEach(panel=>{const active=panel.dataset.featurePanel===id;panel.hidden=!active;panel.classList.toggle('is-active',active);});
  if(updateHash&&location.hash!==`#${id}`)history.replaceState(null,'',`#${id}`);
}
featureTabs.forEach(button=>button.addEventListener('click',()=>{setFeature(button.dataset.featureTab);document.querySelector('#features')?.scrollIntoView({behavior:matchMedia('(prefers-reduced-motion: reduce)').matches?'instant':'smooth',block:'start'});}));
window.addEventListener('hashchange',()=>setFeature(location.hash.slice(1),false));
setFeature(location.hash.slice(1),false);
const menu=document.querySelector('.menu-toggle'),nav=document.querySelector('#navigation');
function closeMenu(){menu?.setAttribute('aria-expanded','false');nav?.classList.remove('open');}
menu?.addEventListener('click',()=>{const open=menu.getAttribute('aria-expanded')!=='true';menu.setAttribute('aria-expanded',String(open));nav.classList.toggle('open',open);});
nav?.querySelectorAll('a').forEach(link=>link.addEventListener('click',closeMenu));
document.addEventListener('keydown',event=>{if(event.key==='Escape')closeMenu();});
document.querySelector('[data-language]')?.addEventListener('click',event=>{const link=event.currentTarget;link.hash=location.hash;});
const dialog=document.querySelector('#image-dialog');
let opener=null;
document.querySelectorAll('[data-zoom]').forEach(button=>button.addEventListener('click',()=>{
  const image=button.querySelector('img'),large=dialog.querySelector('img');large.src=image.src;large.alt=image.alt;opener=button;dialog.showModal();
}));
dialog?.querySelector('button').addEventListener('click',()=>dialog.close());
dialog?.addEventListener('click',event=>{if(event.target===dialog)dialog.close();});
dialog?.addEventListener('close',()=>{opener?.focus({preventScroll:true});dialog.querySelector('img').removeAttribute('src');});
const reduceMotion=matchMedia('(prefers-reduced-motion: reduce)');
featureTabs.forEach((button,index)=>button.addEventListener('keydown',event=>{
  let next=index;
  if(['ArrowDown','ArrowRight'].includes(event.key))next=(index+1)%featureTabs.length;
  else if(['ArrowUp','ArrowLeft'].includes(event.key))next=(index+featureTabs.length-1)%featureTabs.length;
  else if(event.key==='Home')next=0;
  else if(event.key==='End')next=featureTabs.length-1;
  else return;
  event.preventDefault();setFeature(featureTabs[next].dataset.featureTab);featureTabs[next].focus({preventScroll:true});
}));
document.documentElement.classList.add('js-ready');
const reveal=new IntersectionObserver(entries=>entries.forEach(entry=>{if(entry.isIntersecting){entry.target.classList.add('in-view');reveal.unobserve(entry.target);}}),{threshold:.08});
document.querySelectorAll('.ecosystem,.language-story,.workflow,.download,.faq').forEach(element=>{element.classList.add('reveal');reveal.observe(element);});
const stage=document.querySelector('.hero-stage');
stage?.addEventListener('pointermove',event=>{if(reduceMotion.matches||event.pointerType!=='mouse')return;const box=stage.getBoundingClientRect();stage.style.setProperty('--tilt',`${(event.clientY-box.top-box.height/2)/box.height*1.4}deg`);});
stage?.addEventListener('pointerleave',()=>stage.style.setProperty('--tilt','0deg'));
