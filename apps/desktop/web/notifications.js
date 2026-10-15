(() => {
  const invoke=(command)=>window.__TAURI__.core.invoke(command);
  const messages={granted:'应用通知接口可用；实际显示由系统通知设置控制',denied:'系统通知未授权，请在系统通知设置中允许 Aieyes',prompt:'尚未请求系统通知权限',unknown:'无法读取通知权限，请使用测试通知验证'};
  let lastError='';
  async function refresh(){
    const target=document.querySelector('#notification-permission');if(!target)return;
    try {
      const info=await invoke('notifications_info');
      if(!target.isConnected)return;
      target.textContent=messages[info.permission]??messages.unknown;
      const error=document.querySelector('#notification-error');if(error)error.textContent=lastError||info.error||'';
    }catch(error){if(target.isConnected)target.textContent='读取通知状态失败：'+String(error);}
  }
  window.AieyesNotifications={
    settingsHTML:()=>'<section class="card"><h2>系统通知</h2><p id="notification-permission" role="status">正在读取通知权限…</p><button type="button" id="notification-test">发送测试通知</button><p id="notification-error" class="error" role="status"></p><p class="muted tiny">通知的显示与声音遵循系统通知和勿扰模式设置。</p></section>',
    bind(){
      const button=document.querySelector('#notification-test');if(!button)return;
      button.onclick=async()=>{
        button.disabled=true;lastError='';
        try{await invoke('notifications_test');}catch(error){lastError=String(error);}
        finally{button.disabled=false;await refresh();}
      };
      void refresh();
    },
  };
  window.addEventListener('focus',()=>void refresh());
  window.__TAURI__?.event?.listen('desktop:notifications',()=>void refresh()).catch(()=>{});
})();
