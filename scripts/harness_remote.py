"""Read-only Harness v0/v4 projection. Python stdlib + Harness's Node zstd runtime.

Only projected statistics/lifecycle records leave the Node reader. Validate the
entire stream before exposing a batch, including concatenated frame checksums.
"""
import collections
import hashlib
import json
import os
import pathlib
import queue
import shutil
import subprocess
import tempfile
import threading
import time

NODE = r'''
const fs=require('node:fs'),z=require('node:zlib'),{once}=require('node:events');
const [path,mode,rawOptions]=process.argv.slice(1),options=JSON.parse(rawOptions||'{}');
const decoder=new TextDecoder('utf-8',{fatal:true});
function text(v){if(v===undefined||v===null)return '';if(typeof v!=='string'||v.length>4096)throw Error();return v;}
function short(v){return [...text(v)].slice(0,256).join('');}
function number(v){if(v===undefined||v===null)return undefined;if(!Number.isSafeInteger(v)||v<0)throw Error();return v;}
async function emit(row){const line=JSON.stringify(row)+'\n';if(Buffer.byteLength(line)>65536)throw Error();if(!process.stdout.write(line))await once(process.stdout,'drain');}
// Shared by Harness projection and Codex inherited-prefix verification. Node 24
// decodes only the first concatenated frame, so delimit frames before using zlib.
async function decodeZstd(file,consume){
 const iterator=file[Symbol.asyncIterator]();let buffer=Buffer.alloc(0),readBytes=0;
 async function read(n,eof=false){while(buffer.length<n){const next=await iterator.next();if(next.done){if(eof&&!buffer.length)return null;throw Error();}readBytes+=next.value.length;if(readBytes>512*1024*1024)throw Error();buffer=Buffer.concat([buffer,next.value]);}const result=buffer.subarray(0,n);buffer=buffer.subarray(n);return result;}
 for(;;){
  const magic=await read(4,true);if(!magic)break;const id=magic.readUInt32LE();
  if(id>=0x184d2a50&&id<=0x184d2a5f){let remaining=(await read(4)).readUInt32LE();while(remaining){const n=Math.min(65536,remaining);await read(n);remaining-=n;}continue;}
  if(id!==0xfd2fb528)throw Error();
  const descriptor=await read(1),flags=descriptor[0];if(flags&8)throw Error();
  const single=!!(flags&32),sizeFlag=flags>>6;
  const extra=(single?0:1)+[0,1,2,4][flags&3]+[single?1:0,2,4,8][sizeFlag];
  const header=await read(extra),decoder=z.createZstdDecompress({params:{[z.constants.ZSTD_d_windowLogMax]:27}});
  const consumption=(async()=>{for await(const chunk of decoder)await consume(chunk);})();consumption.catch(()=>{});
  const write=chunk=>new Promise((resolve,reject)=>{
   if(decoder.errored){reject(decoder.errored);return;}
   const failed=error=>{decoder.off('error',failed);reject(error);};decoder.once('error',failed);
   decoder.write(chunk,error=>{decoder.off('error',failed);error?reject(error):resolve();});
  });
  try{
   await write(Buffer.concat([magic,descriptor,header]));
   for(;;){const block=await read(3),bits=block.readUIntLE(0,3),type=(bits>>1)&3,size=bits>>>3;if(type===3||size>131072)throw Error();await write(block);if(type===1)await write(await read(1));else if(size)await write(await read(size));if(bits&1)break;}
   if(flags&4)await write(await read(4));decoder.end();await consumption;
  }catch(error){decoder.destroy(error);await consumption.catch(()=>{});throw error;}
 }
}

async function codexPrefix(options){
 const reject=reason=>{throw Object.assign(Error(),{reason});};
 if(typeof z.createZstdDecompress!=='function')reject('decoderUnavailable');
 const metadataOnly=options.limit===undefined,limit=metadataOnly?null:options.limit;
 if(!metadataOnly&&(!Number.isSafeInteger(limit)||limit<0))reject('boundaryInvalid');
 let pieces=[],lineBytes=0,offset=0,total=0,first=true,thread=null,parent=null,tokens=null,timestamp=null;
 const digest=require('node:crypto').createHash('sha256');
 function row(raw){
  let v;try{v=JSON.parse(new TextDecoder('utf-8',{fatal:true}).decode(raw));}catch{reject('invalidRecord');}
  const p=v?.payload||{};
  if(first){
   first=false;thread=p.id||p.session_id;
   if(v?.type!=='session_meta'||typeof thread!=='string'||!thread||thread.length>4096)reject('identityMismatch');
   if(p.history_base&&typeof p.history_base==='object'&&!Array.isArray(p.history_base)){
    parent={thread_id:typeof p.history_base.thread_id==='string'?text(p.history_base.thread_id):null,end_byte_offset:p.history_base.end_byte_offset};
   }
  }
  if(v?.type==='event_msg'&&p.type==='token_count'&&p.info?.total_token_usage&&typeof p.info.total_token_usage==='object'){
   const u=p.info.total_token_usage;
   const values=['input_tokens','output_tokens','cached_input_tokens','cache_write_input_tokens','reasoning_output_tokens'].map(k=>u[k]===undefined?0:u[k]);
   if(u.input_tokens===undefined||values.some(v=>!Number.isSafeInteger(v)||v<0)||values[0]<values[2]+values[3])reject('invalidCounter');
   tokens={input:values[0]-values[2]-values[3],output:values[1],cacheRead:values[2],cacheWrite:values[3],reasoning:values[4]};
   timestamp=v.timestamp;
  }
 }
 async function consume(chunk){
  total+=chunk.length;if(total>512*1024*1024)reject('resourceLimit');
  if(metadataOnly&&!first||!metadataOnly&&offset===limit)return;
  const used=metadataOnly?chunk:chunk.subarray(0,Math.min(chunk.length,limit-offset));
  if(!metadataOnly){offset+=used.length;digest.update(used);}
  let start=0;
  while(start<used.length){
   const index=used.indexOf(10,start),end=index<0?used.length:index,part=used.subarray(start,end);
   lineBytes+=part.length;if(lineBytes+(index>=0?1:0)>16*1024*1024)reject('invalidRecord');
   if(part.length)pieces.push(part);if(index<0)break;
   row(Buffer.concat(pieces,lineBytes));pieces=[];lineBytes=0;start=index+1;
   if(metadataOnly)return;
  }
 }
 const file=fs.createReadStream(path,{highWaterMark:65536});
 try{
  try{await decodeZstd(file,consume);}catch(error){reject(error.reason||'invalidCompressedHistory');}
  if(metadataOnly?first:offset!==limit||lineBytes||limit>0&&first)reject('boundaryInvalid');
  await emit({codexPrefix:'complete',thread,parent,tokens,timestamp,proof:digest.digest('hex')});
 }finally{file.destroy();}
}
async function main(){
 if(mode==='codex-prefix')return codexPrefix(options);
 if(path.endsWith('.zstd')&&typeof z.createZstdDecompress!=='function'){process.exitCode=2;return;}
 const offset=number(options.offset)||0,size=number(options.size)||fs.statSync(path).size;
 const file=fs.createReadStream(path,{highWaterMark:65536,start:offset,end:Math.max(offset,size-1)});
 let pieces=[],lineBytes=0,total=0,first=offset===0,seeded=options.inherited===true,session=options.session||'',version=options.version||4,legacySeed=options.seed||0;
 if(!first)await emit(mode==='history'?{type:'session',version,id:session,isSeeded:seeded,harnessVersion:version}:{kind:'identity',session:short(session),at:0});
 async function row(raw){
  const v=JSON.parse(decoder.decode(raw));if(!v||Array.isArray(v)||typeof v!=='object')throw Error();
  if(first){
   const filename=require('node:path').basename(path);const expected=/^session\.v4\.jsonl(?:\.zstd)?$/.test(filename)?4:/^session\.jsonl(?:\.zstd)?$/.test(filename)?0:-1;
   if(v.type!=='session'||![0,4].includes(v.version)||v.version!==expected)throw Error();
   first=false;version=v.version;legacySeed=version===0?(number(v.seedLength)||0):0;seeded=version===4&&v.isSeeded===true;session=mode==='history'?text(v.id):short(v.id);if(!session)throw Error();
   await emit(mode==='history'?{type:'session',version,id:session,isSeeded:seeded,harnessVersion:version}:{kind:'identity',session,at:0});return;
  }
  if(version===0&&number(v.seq)<legacySeed)return;
  if(v.type==='session')throw Error();
  if(v.type==='session/end-seed'){seeded=false;if(mode==='history')await emit({type:v.type});return;}
  const d=v.data&&typeof v.data==='object'?v.data:{};
  if(mode==='history'){
   if(v.type!=='assistant/message')return;
   const source=d.message?.source||{},u=d.usage||{},usage={};
   for(const k of ['inputTokens','outputTokens','cacheReadTokens','cacheWriteTokens','reasoningTokens','totalTokens']){const n=number(u[k]);if(n!==undefined)usage[k]=n;}
   await emit({type:v.type,harnessVersion:version,seq:number(v.seq),time:number(v.time),data:{turn:number(d.turn),step:number(d.step),message:{source:{kind:text(source.kind),provider:text(source.provider),model:text(source.model)}},usage}});return;
  }
  if(seeded)return;
  let kind={'turn/start':'start','step/start':'thinking','tool/call':'tool','tool/result':'output'}[v.type];
  if(v.type==='turn/end')kind={completed:'complete',cancelled:'interrupted',interrupted:'interrupted',aborted:'interrupted',failed:'failed',error:'failed','max-tokens':'failed',blocked:'PermissionRequest'}[d.reason?.kind];
  if(kind)await emit({kind,session,at:number(v.time)||0,turn:d.turn===undefined?'':String(number(d.turn)),call:short(d.toolCallId||d.callId||d.call_id||d.id),tool:short(d.name)});
 }
 async function consume(chunk){
  total+=chunk.length;if(total>512*1024*1024)throw Error();let start=0;
  while(start<chunk.length){const index=chunk.indexOf(10,start),end=index<0?chunk.length:index;const part=chunk.subarray(start,end);lineBytes+=part.length;if(lineBytes+(index>=0?1:0)>8*1024*1024)throw Error();if(part.length)pieces.push(part);if(index<0)break;await row(Buffer.concat(pieces,lineBytes));pieces=[];lineBytes=0;start=index+1;}
 }
 try{
  if(!path.endsWith('.zstd')){for await(const chunk of file)await consume(chunk);}
  else await decodeZstd(file,consume);
  // A plaintext append in progress is not a committed JSONL record.
  if(first||(path.endsWith('.zstd')&&lineBytes))throw Error();
  await emit({harnessProjection:'complete',offset:path.endsWith('.zstd')?size:offset+total-lineBytes,version,seed:legacySeed,session,inherited:seeded});
 }finally{file.destroy();}
}
main().catch(async error=>{if(mode==='codex-prefix')await emit({codexPrefix:'error',reason:error.reason||'readError'});else process.stderr.write('Harness v0/v4 projection rejected invalid, truncated or oversized data\n');process.exitCode=3;});
'''


def codex_prefix(path, limit=None):
    """Decode on the SSH host; only bounded metering evidence reaches stdout."""
    node = shutil.which('node')
    if not node:
        return dict(reason='decoderUnavailable')
    options = {} if limit is None else dict(limit=limit)
    before = fingerprint(path)
    try:
        result = subprocess.run([node, '-e', NODE, '--', str(path), 'codex-prefix', json.dumps(options)],
                                stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                stderr=subprocess.DEVNULL, timeout=8)
    except subprocess.TimeoutExpired:
        return dict(reason='readError')
    if len(result.stdout) > 65536:
        return dict(reason='invalidRecord')
    try:
        value = json.loads(result.stdout)
    except (ValueError, UnicodeError):
        return dict(reason='readError')
    if result.returncode or value.get('codexPrefix') != 'complete':
        return dict(reason=value.get('reason', 'invalidCompressedHistory'))
    if before != fingerprint(path):
        return dict(reason='readError')
    return value


def fingerprint(path):
    s=os.stat(path)
    values=(s.st_dev,s.st_ino,s.st_size,s.st_mtime_ns,s.st_ctime_ns)
    return 'dsh:'+hashlib.sha256(repr(values).encode()).hexdigest()


def generation(path):
    import re
    name=pathlib.Path(path).name
    if name in ('session.jsonl','session.jsonl.zstd'):return 0
    match=re.fullmatch(r'session\.v([1-9][0-9]*)\.jsonl(?:\.zstd)?',name)
    return int(match[1]) if match else None


def paths(root):
    root=pathlib.Path(root);explicit=root.is_file()
    if explicit and root.is_symlink():raise ValueError('Select the actual Harness session file rather than a symbolic link')
    if explicit:candidates=[root]
    else:
        folder=root/'sessions' if (root/'sessions').is_dir() else root;candidates=[]
        for directory,dirs,names in os.walk(folder,followlinks=False):
            dirs[:]=sorted(n for n in dirs if n!='.aieyes' and not pathlib.Path(directory,n).is_symlink())
            candidates.extend(pathlib.Path(directory,name) for name in sorted(names) if generation(name) is not None)
    selected={}
    for path in candidates:
        if path.is_symlink() or not path.is_file():continue
        version=generation(path)
        if version is None:continue
        if version>4:raise ValueError('Harness session version newer than v4 is unsupported')
        selected.setdefault(path.parent,{}).setdefault(version,[]).append(path)
    result=[]
    for versions in selected.values():
        version=max(versions);files=versions[version]
        if not explicit and len(files)>1:raise ValueError('Harness session has conflicting plaintext and compressed encodings; select one file explicitly')
        if version not in (0,4):raise ValueError('Harness v1-v3 statistics are not yet supported; preserve original records')
        result.append(files[0])
    return sorted(result)


def project(path,mode,timeout=8,cursor=None,checkpoint=None):
    """Yield only after successful decompression and a stable file fingerprint."""
    node=shutil.which('node')
    if not node:raise RuntimeError('Harness SSH reading requires Node with Zstandard support on PATH')
    before=fingerprint(path);stat=os.stat(path);size=stat.st_size
    identity='%s:%s'%(stat.st_dev,stat.st_ino)
    def digest(stream,start,length):
        stream.seek(start);return hashlib.sha256(stream.read(length)).hexdigest()
    old=cursor or {};offset=old.get('offset',0)
    with open(path,'rb') as stream:
        prefix=digest(stream,0,min(size,128))
        resume=(type(offset) is int and 0<offset<=size and size>old.get('size',size)
                and old.get('identity')==identity and old.get('prefix')==prefix
                and old.get('version') in (0,4) and isinstance(old.get('session'),str)
                and digest(stream,max(0,offset-128),min(128,offset))==old.get('anchor'))
    options=dict(offset=offset if resume else 0,size=size)
    if resume:options.update({k:old[k] for k in ('version','seed','session','inherited')})
    process=subprocess.Popen([node,'-e',NODE,'--',str(path),mode,json.dumps(options)],stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL)
    messages=queue.Queue(maxsize=8);stop=threading.Event()
    def reader():
        try:
            while not stop.is_set():
                line=process.stdout.readline(65537)
                if not line:break
                if len(line)>65536 or not line.endswith(b'\n'):raise ValueError('Harness projected record exceeds 64 KiB')
                while not stop.is_set():
                    try:messages.put(line,timeout=.1);break
                    except queue.Full:pass
        except Exception:
            while not stop.is_set():
                try:messages.put(False,timeout=.1);break
                except queue.Full:pass
        finally:
            while not stop.is_set():
                try:messages.put(None,timeout=.1);break
                except queue.Full:pass
    worker=threading.Thread(target=reader,daemon=True);worker.start();deadline=time.monotonic()+timeout
    try:
        with tempfile.SpooledTemporaryFile(max_size=512*1024,mode='w+b') as spool:
            amount=0;complete=None
            while True:
                remaining=deadline-time.monotonic()
                if remaining<=0:raise RuntimeError('Harness metadata projection timed out')
                try:line=messages.get(timeout=min(.1,remaining))
                except queue.Empty:continue
                if line is None:break
                if line is False:raise ValueError('Harness metadata projection failed')
                if complete is not None:raise ValueError('Harness projection returned data after completion')
                if line.startswith(b'{"harnessProjection":'):
                    complete=json.loads(line)
                    if complete.get('harnessProjection')!='complete':raise ValueError('Harness projection completion is invalid')
                    continue
                amount+=len(line)
                if amount>64*1024*1024:raise ValueError('Harness projected metadata exceeds 64 MiB')
                spool.write(line)
            try:status=process.wait(timeout=max(.01,deadline-time.monotonic()))
            except subprocess.TimeoutExpired:raise RuntimeError('Harness metadata projection timed out') from None
            if status==2:raise RuntimeError('Harness SSH reading requires Node with Zstandard support on PATH')
            if status:raise ValueError('Harness session is invalid, truncated, unsupported or oversized')
            if complete is None:raise ValueError('Harness metadata projection did not complete')
            if before!=fingerprint(path):raise RuntimeError('Harness session changed during read; retry')
            end=complete.get('offset')
            if type(end) is not int or not options['offset']<=end<=size:raise ValueError('Harness projection checkpoint is invalid')
            with open(path,'rb') as stream:anchor=digest(stream,max(0,end-128),min(128,end))
            if checkpoint is not None:
                checkpoint.update({k:complete[k] for k in ('offset','version','seed','session','inherited')})
                checkpoint.update(harness=before,identity=identity,size=size,prefix=prefix,anchor=anchor,reset=not resume,readBytes=size-options['offset'])
            spool.seek(0)
            for line in spool:
                try:yield json.loads(line)
                except ValueError:raise ValueError('Harness projected metadata format is invalid') from None
    finally:
        stop.set()
        if process.poll() is None:process.kill()
        process.wait();process.stdout.close();worker.join(timeout=1)


def session(path,cursor,timeout=8):
    signature=fingerprint(path);stat=os.stat(path)
    previous=cursor.get('continuation') or {}
    result=dict(key=str(path),identity=str(stat.st_ino),signature=signature,size=stat.st_size,offset=cursor.get('offset',0),modified=int(stat.st_mtime),reset=False,events=[],hook=False,continuation=previous)
    if cursor.get('signature')==signature and previous.get('harness')==signature:return result
    identity=None;events=collections.deque(maxlen=2048)
    checkpoint={}
    for event in project(path,'state',timeout=timeout,cursor=previous,checkpoint=checkpoint):
        if event['kind']=='identity':identity=event
        else:events.append(event)
    result['events']=([identity] if identity else [])+list(events)
    result.update(reset=checkpoint.pop('reset'),readBytes=checkpoint.pop('readBytes'),offset=checkpoint['offset'],continuation=checkpoint)
    return result
