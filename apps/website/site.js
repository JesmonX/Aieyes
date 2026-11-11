const platformButtons=[...document.querySelectorAll('[data-platform]')];
const reduceMotion=matchMedia('(prefers-reduced-motion: reduce)');
let platform='macos',swapToken=0,stagePlatform=()=>{};
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
  stagePlatform();
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

// The stage is the product in the page. macOS is a set of native captures: each state is an image
// with click targets that lead to other states. Windows is the desktop frontend itself, running in a
// frame on sample data (apps/website/demo/bridge.js). Both follow one tour, so the arrows, the
// window switch and the platform switch always show the same place.
async function startStage(root){
  const response=await fetch(root.dataset.stage);
  if(!response.ok)return;
  const data=await response.json(),$=selector=>root.querySelector(selector);
  const stage=$('.hero-stage'),screen=$('.stage-screen'),image=$('.stage-image'),spots=$('.stage-spots'),status=$('.stage-status');
  const surfaceButtons=[...root.querySelectorAll('[data-surface-tab]')];
  // Phones start with the portrait panel, as the picture without scripts does.
  let surface=matchMedia('(max-width:700px)').matches?'panel':'main',paint=0,swiped=-Infinity;
  const current=Object.fromEntries(Object.entries(data.surfaces).map(([name,group])=>[name,group.initial]));
  const frames={},loaded=new Set();
  const address=name=>`/demo/macos/${data.language}/${name}.webp?v=${data.version}`;
  const group=()=>data.surfaces[surface],state=()=>group().states[current[surface]];
  // The live window carries its own title bar inside the same outer size as the native one.
  const frameHeight=name=>data.surfaces[name].height+(name==='main'?28:0);
  image.closest('picture')?.replaceWith(image);

  function describe(){
    const stops=group().stops,index=stops.findIndex(stop=>stop.id===state().stop),count=number=>String(number).padStart(2,'0');
    $('.tour-count').textContent=`${count(index+1)} / ${count(stops.length)}`;
    $('.tour-title strong').textContent=stops[index].title;$('.tour-text').textContent=stops[index].text;
    $('.stage-note').textContent=platform==='macos'?data.text.native:data.text.live;
  }
  function drawSpots(){
    const {width,height}=group(),focused=spots.contains(document.activeElement)?document.activeElement.getAttribute('aria-label'):null;
    spots.replaceChildren(...state().spots.map(([x,y,w,h,to,label,other])=>{
      const button=document.createElement('button');
      button.type='button';button.className='stage-spot';button.setAttribute('aria-label',label);
      if(!other&&to===current[surface])button.setAttribute('aria-current','true');
      Object.assign(button.style,{left:`${x/width*100}%`,top:`${y/height*100}%`,width:`${w/width*100}%`,height:`${h/height*100}%`});
      button.addEventListener('click',()=>{if(performance.now()-swiped>400)show(other??surface,to);});
      return button;
    }));
    // A keyboard visitor stays on the control they pressed when the next state has it too.
    if(focused)[...spots.children].find(button=>button.getAttribute('aria-label')===focused)?.focus({preventScroll:true});
  }
  function preload(){
    for(const [,,,,to,,other] of state().spots){
      const key=`${other??surface}/${to}`;
      if(!loaded.has(key)){loaded.add(key);new Image().src=address(to);}
    }
  }
  function showNative(){
    const token=++paint,name=current[surface],next=address(name),{width,height}=group();
    $('.stage-title').textContent=`${state().title} — Aieyes`;
    const apply=()=>{
      if(token!==paint)return;
      Object.assign(image,{src:next,width,height,alt:`${state().title} — macOS`,hidden:false});
      drawSpots();setTimeout(()=>{if(token===paint)preload();},300);
    };
    // The next state decodes first, so a click never shows a half-painted window.
    if(image.getAttribute('src')===next){apply();return;}
    const probe=new Image();probe.src=next;probe.decode().then(apply,apply);
  }
  // Where the live frontend cannot start, Windows shows its screenshot of this window.
  function fallback(){
    const [name,width,height]=surface==='main'?['overview',1360,1000]:['panel',450,720];
    Object.assign(image,{src:`/screenshots/${data.language}/windows-${name}.webp`,width,height,alt:`${state().title} — Windows`,hidden:false});
    screen.style.aspectRatio=`${width}/${height}`;
    status.textContent=data.text.failed;status.hidden=false;stage.dataset.mode='image';
  }
  function showLive(){
    let frame=frames[surface];
    if(!frame){
      const element=document.createElement('iframe');
      Object.assign(element,{className:'stage-frame',title:data.text.frame,width:group().width,height:frameHeight(surface)});
      element.src=`/demo/app/${surface==='main'?'index':'floating'}.html?${new URLSearchParams({lang:data.language,theme:'dark',view:JSON.stringify(state().app)})}`;
      frame=frames[surface]={element,ready:false,failed:false,asked:current[surface]};
      screen.append(element);
      // A frame that never reports is a failure too: blocked frames, old engines.
      setTimeout(()=>{if(!frame.ready&&!frame.failed){frame.failed=true;if(platform==='windows'&&frames[surface]===frame)paintStage();}},20000);
    }else if(frame.ready&&frame.asked!==current[surface]){
      frame.asked=current[surface];frame.element.contentWindow.AieyesDemo.go(state().app);
    }
    if(frame.failed){fallback();return;}
    image.hidden=true;status.textContent=data.text.loading;status.hidden=frame.ready;
  }
  function paintStage(){
    stage.dataset.surface=surface;
    surfaceButtons.forEach(button=>button.setAttribute('aria-pressed',String(button.dataset.surfaceTab===surface)));
    for(const [name,frame] of Object.entries(frames))frame.element.hidden=frame.failed||platform!=='windows'||name!==surface;
    status.hidden=true;describe();
    stage.dataset.mode=platform==='macos'?'native':'live';
    screen.style.aspectRatio=`${group().width}/${platform==='macos'?group().height:frameHeight(surface)}`;
    if(platform==='macos')showNative();else{++paint;spots.replaceChildren();showLive();}
    fit();
  }
  function show(nextSurface,name){surface=nextSurface;current[surface]=name;paintStage();}
  function step(delta){const stops=group().stops,index=stops.findIndex(stop=>stop.id===state().stop);show(surface,stops[(index+delta+stops.length)%stops.length].state);}
  // The frame keeps the window's real size and is scaled to the stage, so its layout matches the app.
  function fit(){stage.style.setProperty('--stage-scale',String(screen.clientWidth/group().width));}
  new ResizeObserver(fit).observe(screen);

  // The frontend reports where the visitor went inside the frame; the tour follows.
  const owner=frameWindow=>Object.keys(frames).find(name=>frames[name].element.contentWindow===frameWindow);
  function follow(name,view){
    const states=Object.entries(data.surfaces[name].states),same=key=>([,candidate])=>candidate.app[key]===undefined||candidate.app[key]===view[key];
    const page=states.filter(([,candidate])=>candidate.app.page===view.page&&(candidate.app.tab??null)===(view.tab??null));
    const exact=page.filter(same('sessions')).filter(same('section')).filter(same('agent'));
    // Several native states can be one place in the frontend (a folded card); the current one stays.
    const match=exact.find(([key])=>key===current[name])??exact[0]??page[0];
    if(!match||match[0]===current[name])return;
    current[name]=frames[name].asked=match[0];
    if(name===surface)describe();
  }
  window.AieyesDemoHost={
    ready(frameWindow,view){
      const name=owner(frameWindow);if(!name)return;
      const frame=frames[name];frame.ready=true;
      // The visitor may have moved on while the frame was starting.
      if(frame.asked!==current[name]){frame.asked=current[name];frameWindow.AieyesDemo.go(data.surfaces[name].states[current[name]].app);}
      else follow(name,view);
      if(platform==='windows'&&name===surface)status.hidden=true;
    },
    changed(frameWindow,view){const name=owner(frameWindow);if(name&&frames[name].ready)follow(name,view);},
    failed(frameWindow){const name=owner(frameWindow);if(!name)return;frames[name].failed=true;if(platform==='windows'&&name===surface)paintStage();},
    // The panel's buttons open the main window on the page they name.
    open(frameWindow,view){
      const main=data.surfaces.main,name=view?.target?'settings':view?.page==='analysis'?'analytics':{settings:'settings',prices:'prices','add-host':'machines'}[view?.action];
      show('main',main.states[name]?name:main.initial);
    },
  };

  surfaceButtons.forEach(button=>button.addEventListener('click',()=>show(button.dataset.surfaceTab,current[button.dataset.surfaceTab])));
  root.querySelectorAll('[data-tour-step]').forEach(button=>button.addEventListener('click',()=>step(Number(button.dataset.tourStep))));
  root.addEventListener('keydown',event=>{
    if(event.altKey||event.ctrlKey||event.metaKey||event.shiftKey||!['ArrowLeft','ArrowRight'].includes(event.key))return;
    event.preventDefault();step(event.key==='ArrowLeft'?-1:1);
  });
  // Touch: a sideways swipe over the window moves through the tour.
  let touch=null;
  screen.addEventListener('pointerdown',event=>{touch=event.pointerType==='mouse'?null:{x:event.clientX,y:event.clientY};});
  screen.addEventListener('pointercancel',()=>{touch=null;});
  screen.addEventListener('pointerup',event=>{
    if(!touch)return;
    const x=event.clientX-touch.x,y=event.clientY-touch.y;touch=null;
    if(Math.abs(x)>48&&Math.abs(x)>Math.abs(y)*1.5){swiped=performance.now();step(x<0?1:-1);}
  });
  // The targets show themselves once at the start, and again on a click beside them.
  let hint=0;
  const reveal=()=>{spots.classList.add('is-hinting');clearTimeout(hint);hint=setTimeout(()=>spots.classList.remove('is-hinting'),900);};
  spots.addEventListener('click',event=>{if(event.target===spots)reveal();});
  stagePlatform=paintStage;
  root.classList.add('is-live');paintStage();setTimeout(reveal,1200);
}
const showcase=document.querySelector('[data-stage]');
if(showcase)startStage(showcase).catch(()=>{});
