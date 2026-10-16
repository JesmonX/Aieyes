/* Import aliases, never flatten SSH options or read private key contents. */
(() => {
  let dialog, loading = false;
  async function open() {
    if (loading) return;
    loading = true;
    try {
      const result = await api('hosts.sshConfig.read');
      dialog?.remove(); dialog = document.createElement('dialog');
      dialog.id = 'ssh-config-import'; dialog.setAttribute('aria-labelledby', 'ssh-import-title');
      const existing = new Set(state.settings.hosts.map(h => h.target.toLowerCase()));
      const rows = result.hosts ?? [];
      dialog.innerHTML = `<div class="dialog-head"><h2 id="ssh-import-title">从 SSH 配置导入</h2><button type="button" data-import-close aria-label="关闭">${uiIcon('x')}</button></div><div class="ssh-import-body"><p class="muted source-path">${escapeHTML(result.path)}</p><p class="muted">保留原 SSH 别名，密钥、跳板机等连接选项继续跟随 SSH 配置。地址为预览，未测试连接。</p><input type="search" id="ssh-import-search" placeholder="搜索别名或地址" aria-label="搜索 SSH 服务器"><div class="actions"><button type="button" data-import-all>全选当前结果</button><button type="button" data-import-none>清空选择</button></div><div class="ssh-import-list">${rows.map((host,i) => `<label class="ssh-import-row" data-search="${escapeHTML((host.alias+' '+(host.hostName??'')).toLowerCase())}"><input type="checkbox" data-import-index="${i}" ${existing.has(host.alias.toLowerCase())?'disabled':''}><span><strong>${escapeHTML(host.alias)}</strong><small>${escapeHTML([host.user,host.hostName??'跟随 SSH 配置',host.port].filter(Boolean).join(' · '))}${existing.has(host.alias.toLowerCase())?' · 已添加':''}</small></span></label>`).join('') || '<p class="empty">未找到可导入的 SSH 别名，仍可手动添加主机。</p>'}</div>${(result.warnings??[]).map(w => `<p class="error">${escapeHTML(w)}</p>`).join('')}<p id="ssh-import-error" class="error" role="alert"></p></div><div class="dialog-foot"><button type="button" data-import-close>取消</button><span id="ssh-import-count" role="status"></span><button type="button" class="primary" id="ssh-import-save">导入所选服务器</button></div>`;
      document.body.append(dialog);
      const inputs = () => [...dialog.querySelectorAll('[data-import-index]')];
      const update = () => {const count=inputs().filter(i=>i.checked).length;dialog.querySelector('#ssh-import-count').textContent=`已选 ${count} 台`;dialog.querySelector('#ssh-import-save').disabled=!count;};
      dialog.querySelectorAll('[data-import-close]').forEach(b => b.onclick = () => dialog.close());
      dialog.querySelector('#ssh-import-search').oninput = event => {for(const row of dialog.querySelectorAll('[data-search]'))row.hidden=!row.dataset.search.includes(event.target.value.trim().toLowerCase());};
      dialog.querySelector('[data-import-all]').onclick = () => {inputs().filter(i=>!i.disabled&&!i.closest('[data-search]').hidden).forEach(i=>i.checked=true);update();};
      dialog.querySelector('[data-import-none]').onclick = () => {inputs().forEach(i=>i.checked=false);update();};
      inputs().forEach(i=>i.onchange=update); update();
      dialog.querySelector('#ssh-import-save').onclick = async () => {
        const selected=inputs().filter(i=>i.checked).map(i=>rows[Number(i.dataset.importIndex)]);
        const base=structuredClone(state.settings),next=structuredClone(base),known=new Set(base.hosts.map(h=>h.target.toLowerCase()));
        let added=0;
        for(const host of selected)if(!known.has(host.alias.toLowerCase())){
          known.add(host.alias.toLowerCase()); added++;
          next.hosts.push({id:crypto.randomUUID(),name:host.alias,target:host.alias,authMode:'ssh',username:'',passwordRef:'',port:null,identityFile:'',shell:'/bin/bash',preCommand:'',enabled:true,monitoringEnabled:true,metrics:Object.keys(groups),devices:[],details:Object.keys(detailOptions)});
        }
        dialog.querySelectorAll('button,input').forEach(el=>el.disabled=true);
        dialog.oncancel=event=>event.preventDefault();
        try {
          if(added)await window.AieyesAgentSettings.commit(next,base);
          dialog.close();if($('#editor')?.open)await closeEditor();
          if(state.page==='settings')renderSettings(false);
          notify(`已导入 ${added} 台服务器${selected.length>added?'，重复别名已跳过':''}`);
        }catch(error){dialog.querySelector('#ssh-import-error').textContent=String(error.message??error);}
        finally{dialog.oncancel=null;dialog.querySelectorAll('button,input').forEach(el=>el.disabled=false);inputs().forEach(i=>i.disabled=existing.has(rows[Number(i.dataset.importIndex)].alias.toLowerCase()));update();}
      };
      dialog.showModal();dialog.querySelector('#ssh-import-search').focus();
    } catch(error) { notify('读取 SSH 配置失败：'+String(error.message??error),'error'); }
    finally { loading=false; }
  }
  document.addEventListener('click',event=>{if(event.target.closest('[data-ssh-import]'))void open();});
  window.AieyesSSHConfig={open};
})();
