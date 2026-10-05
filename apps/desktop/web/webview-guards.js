/* Desktop windows must not acquire browser navigation or file-drop behavior. */
document.addEventListener('keydown', event => {
  const shortcut=event.ctrlKey||event.metaKey;
  const browserCommand=event.key==='F5'||(shortcut&&['r','l','p','u','+','-','=','0'].includes(event.key.toLowerCase()))||
    (event.altKey&&['ArrowLeft','ArrowRight','Home'].includes(event.key));
  if(browserCommand)event.preventDefault();
});
document.addEventListener('wheel',event=>{if(event.ctrlKey)event.preventDefault();},{passive:false});
for(const name of ['dragover','drop'])document.addEventListener(name,event=>{
  if(event.dataTransfer?.types.includes('Files'))event.preventDefault();
});
