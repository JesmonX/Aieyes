#!/usr/bin/env python3
"""Actual GTK/WebKitGTK smoke test; run WebKitWebDriver with isolated AIEYES_DATA_DIR.

The driver and app inherit the desktop session. --backend specifies the expected
architecture, not an emulated browser platform. No real accounts are required.
"""
import argparse
import urllib.request, urllib.error, json, time, os, pathlib, signal, base64
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', required=True)
parser.add_argument('--data-dir', required=True, help='Must match WebKitWebDriver AIEYES_DATA_DIR')
parser.add_argument('--backend', choices=['wayland', 'x11'], required=True)
parser.add_argument('--webdriver', default='http://127.0.0.1:4445')
parser.add_argument('--output', required=True)
parser.add_argument('--expect-hour-cycle', choices=['h12', 'h23'])
args = parser.parse_args()
binary = str(pathlib.Path(args.binary).resolve())
data_dir = str(pathlib.Path(args.data_dir).resolve())
assert str(pathlib.Path(os.environ.get('AIEYES_DATA_DIR', '')).resolve()) == data_dir, 'Set isolated AIEYES_DATA_DIR before starting this script and WebKitWebDriver'
base = args.webdriver.rstrip('/')
output = pathlib.Path(args.output)
output.mkdir(parents=True, exist_ok=True)
session = None
def call(method,path,value=None):
 request=urllib.request.Request(base+path,data=json.dumps(value).encode() if value is not None else None,method=method,headers={'Content-Type':'application/json'})
 try:
  response=json.load(urllib.request.urlopen(request,timeout=65))
 except urllib.error.HTTPError as e:raise RuntimeError(e.read().decode())
 if isinstance(response.get('value'),dict) and response['value'].get('error'):raise RuntimeError(response)
 return response['value']
def js(script):return call('POST',f'/session/{session}/execute/sync',{'script':script,'args':[]})
def asyncjs(expression):return call('POST',f'/session/{session}/execute/async',{'script':f'const done=arguments[arguments.length-1];Promise.resolve({expression}).then(value=>done({{value}}),error=>done({{error:String(error)}}));','args':[]})
def invoke(command,args={}):
 result=asyncjs(f'window.__TAURI__.core.invoke({json.dumps(command)},{json.dumps(args)})')
 if 'error' in result:raise RuntimeError(result)
 return result.get('value')
def processes():
 result=[]
 for p in pathlib.Path('/proc').iterdir():
  if not p.name.isdigit():continue
  try:
   if str((p/'exe').resolve()) == binary:
    env = (p/'environ').read_bytes().split(b'\0')
    if ('AIEYES_DATA_DIR=' + data_dir).encode() in env: result.append((int(p.name), env))
  except (PermissionError,FileNotFoundError,ProcessLookupError):pass
 return result
try:
 for _ in range(50):
  try:call('GET','/status');break
  except Exception:time.sleep(.2)
 value=call('POST','/session',{'capabilities':{'alwaysMatch':{'webkitgtk:browserOptions':{'binary':binary}}}})
 session=value['sessionId'];call('POST',f'/session/{session}/timeouts',{'script':60000})
 for _ in range(100):
  if js('return !!window.AieyesApp && !!window.__TAURI__'):break
  time.sleep(.2)
 asyncjs('AieyesApp.ready');time.sleep(3)
 info=invoke('desktop_info');print('INFO',json.dumps(info,ensure_ascii=False),flush=True)
 assert info['platform']=='linux'
 if args.expect_hour_cycle:
  for pid,env in processes():print('CLOCK ENV',pid,[v.decode(errors='replace') for v in env if v.startswith((b'XDG_CURRENT_DESKTOP=',b'GSETTINGS_BACKEND=',b'XDG_CONFIG_HOME=',b'HOME='))],flush=True)
  assert info['hourCycle']==args.expect_hour_cycle
 rows=processes();assert rows, 'Driver app did not inherit the isolated AIEYES_DATA_DIR';children=[pid for pid,env in rows if any(v.startswith(b'AIEYES_X11_HELPER=') for v in env)]
 print('PROCESSES',[(pid,'helper' if pid in children else 'parent') for pid,_ in rows],flush=True)
 if args.backend == 'wayland':assert len(children)==1
 else:assert not children
 invoke('desktop_panel',{'open':True});time.sleep(4)
 assert invoke('desktop_info')['panelOpen']
 invoke('desktop_panel_pin',{'pinned':True});assert invoke('desktop_info')['panelPinned']
 invoke('desktop_action',{'action':'refresh-floating'});time.sleep(5)
 assert invoke('desktop_info')['panelOpen']
 invoke('desktop_panel',{'open':False});assert not invoke('desktop_info')['panelOpen']
 # A fresh helper must reconnect once; old process generation can no longer affect it.
 if children:
  os.kill(children[0],signal.SIGKILL);time.sleep(5)
  after=[pid for pid,env in processes() if any(v.startswith(b'AIEYES_X11_HELPER=') for v in env)]
  assert len(after)==1 and after[0]!=children[0],after
  invoke('desktop_panel',{'open':True});time.sleep(3);assert invoke('desktop_info')['panelOpen']
 # Save main logical geometry and exercise narrow settings in the actual WebKitGTK engine.
 call('POST',f'/session/{session}/window/rect',{'width':900,'height':700})
 js("AieyesApp.openSettings({category:'general',section:'desktop'});return true")
 time.sleep(1)
 assert js('return document.documentElement.scrollWidth<=innerWidth')
 png=call('GET',f'/session/{session}/screenshot');(output/'linux-native.png').write_bytes(base64.b64decode(png))
 if children:
  # A second failure exhausts the one-restart policy and exposes the main window.
  os.kill(after[0], signal.SIGKILL);time.sleep(4)
  assert not [pid for pid,env in processes() if any(v.startswith(b'AIEYES_X11_HELPER=') for v in env)]
  assert invoke('desktop_info')['effectiveMode'] == 'main'
  assert asyncjs('__TAURI__.window.getCurrentWindow().isVisible()')['value']
 print('PASS native Linux startup, RPC, panel/pin/recreate, helper restart/fallback, narrow settings (' + args.backend + ')',flush=True)
finally:
 for pid,env in processes():
  if not any(v.startswith(b'AIEYES_X11_HELPER=') for v in env):
   try:os.kill(pid,signal.SIGTERM)
   except ProcessLookupError:pass
 time.sleep(1)
 for pid,_ in processes():
  try:os.kill(pid,signal.SIGTERM)
  except ProcessLookupError:pass
 if session:
  try:call('DELETE',f'/session/{session}')
  except Exception:pass
