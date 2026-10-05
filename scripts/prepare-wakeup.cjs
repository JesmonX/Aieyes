// Bundle a native headless runner without a WebView or a Python dependency.
const fs=require('node:fs'),path=require('node:path'),{spawnSync}=require('node:child_process');
const root=path.resolve(__dirname,'..'),debug=process.argv.includes('--debug');
const args=['build','--manifest-path',path.join(root,'Cargo.toml'),'--bin','aieyes-core','--locked'];
if(!debug)args.push('--release');
const build=spawnSync('cargo',args,{cwd:root,stdio:'inherit'});
if(build.error)throw build.error;if(build.status!==0)process.exit(build.status||1);
const name=process.platform==='win32'?'aieyes-core.exe':'aieyes-core',dest=path.join(root,'apps/desktop/src-tauri/binaries');
fs.mkdirSync(dest,{recursive:true});fs.copyFileSync(path.join(root,'target',debug?'debug':'release',name),path.join(dest,name));
