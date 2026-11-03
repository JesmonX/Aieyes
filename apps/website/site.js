const platformButtons=[...document.querySelectorAll('[data-platform]')];
const reduceMotion=matchMedia('(prefers-reduced-motion: reduce)');
let platform='macos',swapToken=0;
try{if(sessionStorage.getItem('aieyes.previewPlatform')==='windows')platform='windows';}catch{}
// Keeps the content-version query; <source> carries the phone-sized screenshot.
function previewPath(element,value){
  const current=element.getAttribute(element.tagName==='SOURCE'?'srcset':'src');
  return `/screenshots/${document.documentElement.lang}/${value}-${element.dataset.preview}.webp${new URL(current,location.href).search}`;
}
function inView(element){const box=element.getBoundingClientRect();return box.width>0&&box.bottom>0&&box.top<innerHeight;}
function setPlatform(value,animate=true){
  platform=value;document.documentElement.dataset.platform=value;
  platformButtons.forEach(button=>button.setAttribute('aria-pressed',String(button.dataset.platform===value)));
  const token=++swapToken;
  document.querySelectorAll('img[data-preview]').forEach(image=>{
    const source=image.parentElement.tagName==='PICTURE'?image.parentElement.querySelector('source[data-preview]'):null;
    const apply=()=>{if(source)source.srcset=previewPath(source,value);image.src=previewPath(image,value);};
    image.alt=image.alt.replace(/ — (?:macOS|Windows)$/,'')+` — ${value==='macos'?'macOS':'Windows'}`;
    const next=source&&matchMedia(source.media).matches?previewPath(source,value):previewPath(image,value);
    if(image.getAttribute('src')===previewPath(image,value)&&(!source||source.getAttribute('srcset')===previewPath(source,value)))return;
    // Off-screen, lazy or reduced-motion images switch at once; visible ones decode first, then crossfade.
    if(!animate||reduceMotion.matches||!image.complete||!inView(image)){image.classList.remove('is-swapping');apply();return;}
    const preload=new Image();preload.src=next;
    preload.decode().catch(()=>{}).then(()=>{
      if(token!==swapToken)return;
      image.classList.add('is-swapping');
      setTimeout(()=>{
        if(token!==swapToken)return;
        apply();image.decode().catch(()=>{}).then(()=>{if(token===swapToken)image.classList.remove('is-swapping');});
      },150);
    });
  });
  try{sessionStorage.setItem('aieyes.previewPlatform',value);}catch{}
}
platformButtons.forEach(button=>button.addEventListener('click',()=>setPlatform(button.dataset.platform)));
setPlatform(platform,false);
const featureTabs=[...document.querySelectorAll('[data-feature-tab]')];
const featurePanels=[...document.querySelectorAll('[data-feature-panel]')];
const tabList=document.querySelector('.workspace-tabs');
const compactFeatures=matchMedia('(max-width:700px), (max-width:1000px) and (pointer:coarse)');
function updateTabFade(){
  if(!tabList)return;
  const end=tabList.scrollWidth-tabList.clientWidth;
  tabList.classList.toggle('fade-start',end>1&&tabList.scrollLeft>1);
  tabList.classList.toggle('fade-end',end>1&&tabList.scrollLeft<end-1);
}
tabList?.addEventListener('scroll',updateTabFade,{passive:true});
function revealFeatureTab(){
  updateTabFade();
  const active=featureTabs.find(button=>button.getAttribute('aria-selected')==='true');
  if(!active||!tabList||tabList.scrollWidth<=tabList.clientWidth)return;
  const button=active.getBoundingClientRect(),list=tabList.getBoundingClientRect();
  tabList.scrollLeft+=button.left+button.width/2-list.left-list.width/2;
  updateTabFade();
}
function updateFeatureLayout(){
  tabList?.setAttribute('aria-orientation',compactFeatures.matches?'horizontal':'vertical');
  revealFeatureTab();
}
compactFeatures.addEventListener('change',updateFeatureLayout);
window.addEventListener('resize',revealFeatureTab);
function setFeature(value,updateHash=true){
  const id=featureTabs.some(button=>button.dataset.featureTab===value)?value:'overview';
  featureTabs.forEach(button=>{const active=button.dataset.featureTab===id;button.setAttribute('aria-selected',String(active));button.tabIndex=active?0:-1;});
  featurePanels.forEach(panel=>{const active=panel.dataset.featurePanel===id;panel.inert=!active;panel.classList.toggle('is-active',active);});
  revealFeatureTab();
  if(updateHash&&location.hash!==`#${id}`)history.replaceState(null,'',`#${id}`);
}
featureTabs.forEach(button=>button.addEventListener('click',()=>{setFeature(button.dataset.featureTab);document.querySelector('#features')?.scrollIntoView({behavior:matchMedia('(prefers-reduced-motion: reduce)').matches?'instant':'smooth',block:'start'});}));
window.addEventListener('hashchange',()=>setFeature(location.hash.slice(1),false));
setFeature(location.hash.slice(1),false);
updateFeatureLayout();
const menu=document.querySelector('.menu-toggle'),nav=document.querySelector('#navigation');
function closeMenu(){menu?.setAttribute('aria-expanded','false');nav?.classList.remove('open');}
menu?.addEventListener('click',()=>{const open=menu.getAttribute('aria-expanded')!=='true';menu.setAttribute('aria-expanded',String(open));nav.classList.toggle('open',open);});
nav?.querySelectorAll('a').forEach(link=>link.addEventListener('click',closeMenu));
document.addEventListener('keydown',event=>{if(event.key==='Escape')closeMenu();});
document.querySelector('[data-language]')?.addEventListener('click',event=>{const link=event.currentTarget;link.hash=location.hash;});
const dialog=document.querySelector('#image-dialog');
let opener=null;
document.querySelectorAll('[data-zoom]').forEach(button=>button.addEventListener('click',()=>{
  const image=button.querySelector('img'),large=dialog.querySelector('img');large.src=image.currentSrc||image.src;large.alt=image.alt;opener=button;dialog.showModal();
}));
dialog?.querySelector('button').addEventListener('click',()=>dialog.close());
dialog?.addEventListener('click',event=>{if(event.target===dialog)dialog.close();});
dialog?.addEventListener('close',()=>{opener?.focus({preventScroll:true});dialog.querySelector('img').removeAttribute('src');});
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
document.querySelectorAll('.provider-list,.steps,.downloads,.faq-list').forEach(group=>{group.classList.add('stagger');[...group.children].forEach((child,index)=>child.style.setProperty('--i',index));});
const stage=document.querySelector('.hero-stage');
stage?.addEventListener('pointermove',event=>{if(reduceMotion.matches||event.pointerType!=='mouse')return;const box=stage.getBoundingClientRect();stage.style.setProperty('--tilt',`${(event.clientY-box.top-box.height/2)/box.height*1.4}deg`);});
stage?.addEventListener('pointerleave',()=>stage.style.setProperty('--tilt','0deg'));
