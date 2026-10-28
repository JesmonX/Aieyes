/* Desktop windows must not acquire browser navigation, find, zoom, select-all or file-drop behavior.
   Only editable fields and selected text keep a context menu; on Windows the native layer trims that
   menu to editing commands (desktop.rs keep_edit_commands). */
{
  const editable=target=>target instanceof Element&&!!target.closest('textarea,[contenteditable]:not([contenteditable=false]),input:not([type=checkbox],[type=radio],[type=range],[type=button],[type=submit],[type=reset],[type=color],[type=file],[type=image])');
  const selected=target=>{
    const selection=getSelection();
    return target instanceof Element&&!!selection&&!selection.isCollapsed&&selection.toString().trim()!==''&&
      selection.containsNode(target,true)&&!target.closest('a[href],img');
  };
  document.addEventListener('keydown', event => {
    const shortcut=event.ctrlKey||event.metaKey,key=event.key?.toLowerCase();
    const browserCommand=event.key==='F5'||event.key==='F3'||(shortcut&&['r','l','p','u','f','g','+','-','=','0'].includes(key))||
      (event.altKey&&['ArrowLeft','ArrowRight','Home'].includes(event.key))||(shortcut&&key==='a'&&!editable(event.target));
    if(browserCommand)event.preventDefault();
  });
  document.addEventListener('contextmenu',event=>{if(!editable(event.target)&&!selected(event.target))event.preventDefault();});
  document.addEventListener('wheel',event=>{if(event.ctrlKey)event.preventDefault();},{passive:false});
  for(const name of ['dragover','drop'])document.addEventListener(name,event=>{
    if(event.dataTransfer?.types.includes('Files'))event.preventDefault();
  });
}
