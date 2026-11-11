// Demo assets: the desktop frontend with its synthetic backend, and the native macOS captures.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {createHash} from 'node:crypto';
import {execFileSync} from 'node:child_process';

// Stops of the tour, in sidebar order. Each lists the native captures that belong to it: the first is
// the one the stop opens, the others are places the live frontend can reach within the stop. Every
// native state belongs to exactly one stop.
export const TOUR={
  main:[
    {id:'overview',mac:['overview','overview-collapsed']},
    {id:'sessions',mac:['overview-sessions','overview-sessions-collapsed']},
    {id:'analytics',mac:['analytics']},
    {id:'servers',mac:['servers']},
    {id:'accounts',mac:['accounts','accounts-claude','accounts-antigravity']},
    {id:'machines',mac:['machines']},
    {id:'prices',mac:['prices']},
    {id:'wakeups',mac:['wakeups']},
    {id:'settings',mac:['settings','settings-desktop','settings-network','settings-refresh','settings-notifications','settings-advanced','settings-updates']},
    {id:'setup',mac:['setup']},
  ],
  panel:[
    {id:'panel',mac:['panel']},
    {id:'panel-quotas',mac:['panel-quotas']},
    {id:'panel-servers',mac:['panel-servers']},
  ],
};
// Native settings tabs and the categories the desktop frontend calls them.
const SETTINGS_TABS={integrations:'integrations',servers:'hosts',prices:'prices',wakeups:'wakeups',general:'general'};
// The same place in the desktop frontend, as bridge.js `go` takes it.
function appView(view,where){
  if(view.page==='agent')return {page:'agent',sessions:view.sessions};
  if(['analysis','servers','setup'].includes(view.page))return {page:view.page};
  const tab=view.page==='settings'?SETTINGS_TABS[view.tab]:null;
  if(!tab)throw new Error(`${where}: no desktop page for ${JSON.stringify(view)}`);
  return {page:'settings',tab,...(tab==='integrations'?{agent:view.agent}:tab==='general'?{section:view.general}:{})};
}

function seed(root,language,version){
  const python=process.env.AIEYES_PREVIEW_PYTHON||'python3',zh=language==='zh-CN';
  const raw=JSON.parse(execFileSync(python,[path.join(root,'scripts/website/fixture.py'),'--fixture'],{encoding:'utf8',maxBuffer:64*1024*1024,env:{...process.env,AIEYES_LANGUAGE:language}}));
  const {settings,dashboard}=raw,stamp=dashboard.generatedAt,local=zh?'本机':'Local device';
  // Regression filler of the shared fixture; a visitor's price page lists real models only.
  settings.modelMappings={};settings.localDeviceName=zh?'工作站':'Workstation';
  // One total per day, oldest first; the frontend's filters narrow these in the page.
  const totals=dashboard.heatmap.map(day=>day.total);
  dashboard.trendDays.forEach((day,index)=>{totals[totals.length-dashboard.trendDays.length+index]=day.total;});
  const all=dashboard.models.reduce((sum,model)=>({total:sum.total+model.total,cost:sum.cost+model.cost}),{total:0,cost:0});
  const usage={
    totals,rate:dashboard.summary.cost/dashboard.summary.total,
    split:Object.fromEntries(['input','output','cacheRead','cacheWrite'].map(name=>[name,dashboard.summary.tokens[name]/dashboard.summary.total])),
    models:dashboard.models.map(model=>({key:model.key,provider:/claude/.test(model.key)?'claude':/gemini/.test(model.key)?'antigravity':'codex',ratio:model.total/all.total,costShare:model.cost/all.cost})),
  };
  // A running turn counts up from these starts once the page's synthetic day begins.
  const turns=[{workspace:'/demo/projects/aieyes-web',turnStartedAt:stamp*1000-192000},{workspace:'/demo/projects/api-gateway',turnStartedAt:stamp*1000-754000},{workspace:'/srv/train/vision-lab',turnStartedAt:stamp*1000-67000}];
  const sessions=raw.sessions.sessions.map((session,index)=>({...session,source:settings.sources.find(source=>source.id===session.sourceId)?.name??session.source,machine:session.machineId==='local'?local:session.machine,...turns[index]}));
  for(const row of raw.wakeups){
    if(zh)row.task.name='工作日唤醒';
    if(row.deployment){row.deployment.task=row.task;row.deployment.target=local;}
  }
  const pet={id:'yuexinmiao1',displayName:zh?'月薪喵':'Yuexinmiao',contentHash:'website-demo',spritePath:'website-demo',spriteVersionNumber:1,rows:3,cellWidth:192,cellHeight:208,libraryId:'website-yuexinmiao',creator:'kiffin',source:'codex-pet.org',importedAt:'2026-10-21T12:00:00Z'};
  const data={
    platform:'windows',scenario:'website',version,stamp,settings,prices:raw.prices,hosts:raw.hosts,wakeups:raw.wakeups,network:raw.network,sessions,usage,
    dashboard:Object.fromEntries(['generatedAt','summary','quotas','quotaOrder','quotaEstimates','creditEstimates','pricingGaps'].map(key=>[key,dashboard[key]])),
    petLibrary:{pets:[pet],selectedLibraryId:pet.libraryId,warnings:[]},
  };
  const text=JSON.stringify(data);
  // The fixture may embed local render paths (the pet sheet); none of that is published.
  if(text.includes(os.homedir())||/[/\\]Users[/\\]|[/\\]home[/\\]/.test(text))throw new Error('Demo data contains a local path');
  return {data,text};
}

// Returns what the page needs: the native states for each language, and the image version.
export function buildDemo({root,source,out}){
  const web=path.join(root,'apps/desktop/web'),app=path.join(out,'demo/app');
  const version=JSON.parse(fs.readFileSync(path.join(root,'apps/desktop/src-tauri/tauri.conf.json'),'utf8')).version;
  fs.rmSync(path.join(out,'demo/app'),{recursive:true,force:true});fs.mkdirSync(app,{recursive:true});
  // The frontend as shipped. Bundled fonts stay with the installers: their license covers unmodified
  // copies inside software, so the demo neither subsets nor serves them and uses the visitor's system fonts.
  const inject='<script src="demo-config.js"></script><script src="demo-model.js"></script><script src="demo-bridge.js"></script>';
  for(const name of fs.readdirSync(web)){
    const file=path.join(web,name);
    if(!fs.statSync(file).isFile()||!/\.(?:js|css|html|png)$/.test(name))continue;
    if(name==='tokens.css'){
      const css=fs.readFileSync(file,'utf8'),faces=css.match(/^@font-face \{[^\n]*\}\n/gm)??[];
      if(faces.length!==3||!faces.every(face=>face.includes('fonts/HarmonyOS_Sans_SC_')))throw new Error('Unexpected bundled font faces in tokens.css');
      fs.writeFileSync(path.join(app,name),faces.reduce((text,face)=>text.replace(face,''),css));
    }else if(name.endsWith('.html')){
      const html=fs.readFileSync(file,'utf8');
      if(html.split('<head>').length!==2||html.split('</head>').length!==2)throw new Error('Unexpected document head in '+name);
      fs.writeFileSync(path.join(app,name),html.replace('<head>','<head><meta name="robots" content="noindex">'+inject).replace('</head>','<link rel="stylesheet" href="demo.css"></head>'));
    }else fs.copyFileSync(file,path.join(app,name));
  }
  for(const name of ['index.html','floating.html'])if(!fs.existsSync(path.join(app,name)))throw new Error('Missing desktop page: '+name);
  fs.copyFileSync(path.join(root,'scripts/desktop-preview/model.js'),path.join(app,'demo-model.js'));
  fs.copyFileSync(path.join(source,'demo/bridge.js'),path.join(app,'demo-bridge.js'));
  fs.copyFileSync(path.join(source,'demo/app.css'),path.join(app,'demo.css'));
  let stamp;
  for(const language of ['en','zh-CN']){
    const {data,text}=seed(root,language,version);
    if(stamp!==undefined&&stamp!==data.stamp)throw new Error('Demo languages disagree on the synthetic time');
    stamp=data.stamp;fs.writeFileSync(path.join(out,'demo',`data.${language}.json`),text);
  }
  fs.writeFileSync(path.join(app,'demo-config.js'),`window.AieyesDemoConfig=${JSON.stringify({stamp,data:'../data.{language}.json',pet:'/pet-spritesheet.webp'})};\n`);

  const manifest=JSON.parse(fs.readFileSync(path.join(source,'demo/macos.json'),'utf8')),hash=createHash('sha256');
  hash.update(JSON.stringify(manifest));
  for(const [surface,group] of Object.entries(manifest)){
    if(!TOUR[surface])throw new Error('No tour for the native surface: '+surface);
    for(const stop of TOUR[surface])for(const state of stop.mac)if(!group.states[state])throw new Error(`Tour stop ${surface}/${stop.id} needs the native state ${state}`);
    if(!group.states[group.initial])throw new Error('Missing initial native state: '+group.initial);
    for(const name of Object.keys(group.states)){
      if(TOUR[surface].filter(stop=>stop.mac.includes(name)).length!==1)throw new Error(`${surface}/${name}: a native state belongs to exactly one tour stop`);
      for(const language of ['en','zh-CN']){
        const image=path.join(source,'public/demo/macos',language,name+'.webp');
        if(!fs.existsSync(image))throw new Error('Missing native state image: '+image);
        hash.update(fs.readFileSync(image));
      }
    }
  }
  const imageVersion=hash.digest('hex').slice(0,12);
  // One language per page: its titles, with each state's place in the tour and in the frontend.
  const forLanguage=language=>({
    version:imageVersion,language,
    surfaces:Object.fromEntries(Object.entries(manifest).map(([surface,group])=>[surface,{
      width:group.width,height:group.height,initial:group.initial,
      stops:TOUR[surface].map(stop=>({id:stop.id,state:stop.mac[0]})),
      states:Object.fromEntries(Object.entries(group.states).map(([name,state])=>[name,{
        title:state.title[language],stop:TOUR[surface].find(stop=>stop.mac.includes(name)).id,app:appView(state.view,`${surface}/${name}`),
      }])),
    }])),
  });
  return {imageVersion,forLanguage};
}
