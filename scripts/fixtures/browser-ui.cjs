// Shared synthetic IPC; no personal settings, hosts or accounts are read.
module.exports = () => {
  const groups=['cpu','memory','gpu','filesystems','disk','network'];
  const details=['cpuTimes','memoryCache','swap','fsAvailable','fsType','inodes','diskIops','diskBusy','networkTotals','networkErrors','gpuMemory','gpuThermals'];
  const tokens={input:240000,output:180000,cacheRead:160000,cacheWrite:20000};
  const usage={tokens,total:600000,events:128,pricedTokens:600000,cost:2.46};
  const day='2026-10-03';
  const history=Array.from({length:365},(_,i)=>({...usage,key:new Date(Date.UTC(2025,9,4+i)).toISOString().slice(0,10),total:i%5?100000+(i*71239)%850000:0}));
  const trend=history.slice(-7);
  window.fixtureSettings={accounts:[],sources:[],hosts:[{id:'host1',name:'训练服务器',target:'gpu-lab',port:null,identityFile:'',shell:'/bin/bash',preCommand:'',enabled:true,metrics:groups,devices:[],details}],proxy:{mode:'system',url:''},refreshSeconds:300,serverRefreshSeconds:10,modelMappings:{}};
  window.saved=[]; window.discoveryFails=false; window.nativeCalls=[]; window.engineCalls=[]; window.settingsSaveFails=false; window.quotaFails=false; let maximized=false;
  window.uiTimers=[];
  window.setInterval=(fn,ms)=>{window.uiTimers.push({fn,ms});return window.uiTimers.length;};
  window.desktopInfo={platform:'windows',material:'opaque',mode:'auto',effectiveMode:'floating',page:'agent',sessions:[],summary:'暂无活跃会话',panelOpen:false,panelPinned:false,hidden:false};
  window.desktopCalls=[]; window.__listeners={};
  window.__TAURI__={
    event:{async listen(name,fn){window.__listeners[name]=fn;return ()=>{};}},
    window:{getCurrentWindow:()=>({async isMaximized(){return maximized;},async toggleMaximize(){maximized=!maximized;window.nativeCalls.push('maximize');},async minimize(){window.nativeCalls.push('minimize');},async close(){window.nativeCalls.push('close');},async onResized(){return ()=>{};}})},
    core:{async invoke(command,args){
      if(command==='updates_info'||command==='updates_check')return {currentVersion:'0.1.1',latestVersion:'0.1.1',phase:command==='updates_check'?'current':'idle',message:command==='updates_check'?'当前已是最新版本 v0.1.1':'',automatic:true,prompt:false};
      if(command==='desktop_info')return structuredClone(window.desktopInfo);
      if(command==='desktop_detail'||command==='desktop_panel'||command==='desktop_panel_pin'||command==='desktop_action'){
        window.desktopCalls.push({command,args:structuredClone(args)});
        if(command==='desktop_panel')window.desktopInfo.panelOpen=args.open;
        if(command==='desktop_panel_pin')window.desktopInfo.panelPinned=args.pinned;
        return structuredClone(window.desktopInfo);
      }
      if(command!=='engine_call')return {};
      window.engineCalls.push({method:args.method,params:structuredClone(args.params)});
      switch(args.method){
        case 'hello':return {version:'0.1.1'};
        case 'settings.get':return structuredClone(window.fixtureSettings);
        case 'accounts.status.get':case 'accounts.cleanup.list':return [];
        case 'sources.configure': {
          const next=structuredClone(window.fixtureSettings),item=args.params.source,i=next.sources.findIndex(s=>s.id===item.id);if(i<0)next.sources.push(item);else next.sources[i]=item;
          window.fixtureSettings=next;window.saved.push(structuredClone(next));return structuredClone(next);
        }
        case 'settings.patch':
          args.params=args.params.settings;
          if(window.settingsSaveFails)throw new Error('磁盘不可写');
          if(args.params.sources.some(s=>!['antigravity','deepseek'].includes(s.provider)&&!s.path.trim()))throw new Error('请输入数据目录');
          if(args.params.refreshSeconds<10||args.params.refreshSeconds>86400)throw new Error('Agent 刷新间隔范围为 10–86400 秒');
          window.fixtureSettings=structuredClone(args.params);window.saved.push(structuredClone(args.params));return structuredClone(args.params);
        case 'credentials.save':return {path:`test-credentials/${args.params.sourceId}.key`};
        case 'quotas.schedule':return {nextDueAt:window.fixtureSettings.accounts.some(a=>a.quotaEnabled&&!a.archived)?(window.fixtureQuotaAttempt??Date.now()/1000)+300:null};
        case 'quotas.refresh':window.fixtureQuotaAttempt=Date.now()/1000;if(window.quotaFails)throw new Error('限额连接失败');return [];
        case 'sources.scan':return {};
        case 'dashboard':return {summary:window.fixtureUsage??usage,quotas:[],models:[{...usage,key:'Claude Sonnet'}],trendDays:trend,dayModels:trend.map(row=>({day:row.key,model:'Claude Sonnet',usage:row})),heatmap:history,sources:window.fixtureSources??[],pricingGaps:[]};
        case 'hosts.sample':return window.fixtureHosts??[];
        case 'prices.list':return window.fixturePrices??[{id:'anthropic/claude-sonnet',name:'Claude Sonnet',input:0.000003,output:0.000015},{id:'openai/gpt-test',name:'GPT Test',input:0.000001,output:0.000002}];
        case 'prices.save':window.fixturePrices=args.params.prices;return {};
        case 'prices.sync':return {};
        case 'hosts.discover':
          if(window.discoveryFails)throw new Error('连接失败');
          return {cpu:[{id:'cpu'},...Array.from({length:256},(_,i)=>({id:`cpu${i}`}))],network:[{id:'eth0'},{id:'eth1'},{id:'ib0',name:'高速网卡'}],filesystems:Array.from({length:100},(_,i)=>({id:`/data/挂载点-${i}-long-name`,type:'ext4'})),gpu:[{id:'0',name:'GPU'}],disk:[{id:'nvme0n1'}],errors:{}};
        default:throw new Error(`Unexpected call: ${args.method}`);
      }
    }},
  };
};
