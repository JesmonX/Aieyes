(() => {
  const invoke=(command)=>window.__TAURI__.core.invoke(command);
  const messages={granted:'通知接口可用，请发送测试通知检查显示',denied:'系统通知未授权，请在系统通知设置中允许 Aieyes',prompt:'尚未请求系统通知权限',unknown:'无法读取通知权限，请使用测试通知验证'};
  let lastError='',lastNotice='';
  async function refresh(){
    const target=document.querySelector('#notification-permission');if(!target)return;
    try {
      const info=await invoke('notifications_info');
      if(!target.isConnected)return;
      target.textContent=messages[info.permission]??messages.unknown;
      const error=document.querySelector('#notification-error');AieyesUI.showError(error,lastError||info.error||'','通知操作失败');
      const notice=document.querySelector('#notification-test-result');if(notice)notice.textContent=lastNotice;
    }catch(error){if(target.isConnected)target.textContent='读取通知状态失败：'+String(error);}
  }
  window.AieyesNotifications={
    settingsHTML:()=>'<section class="card"><h2>系统通知</h2><p id="notification-permission" role="status">正在读取通知权限…</p><button type="button" id="notification-test">发送测试通知</button><p id="notification-test-result" role="status"></p><div id="notification-error" class="error" role="status"></div><p class="muted tiny">通知显示和声音由系统通知及勿扰设置控制。</p></section>',
    bind(){
      const button=document.querySelector('#notification-test');if(!button)return;
      button.onclick=async()=>{
        button.disabled=true;lastError='';lastNotice='';
        try{await invoke('notifications_test');lastNotice='测试通知已提交，请检查系统显示。';}catch(error){lastError=String(error);}
        finally{button.disabled=false;await refresh();}
      };
      void refresh();
    },
  };
  window.addEventListener('focus',()=>void refresh());
  window.__TAURI__?.event?.listen('desktop:notifications',()=>void refresh()).catch(()=>{});
})();
