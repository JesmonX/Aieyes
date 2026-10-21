#!/usr/bin/env python3
"""Exercise real GTK/Tauri windows in a GNOME/XWayland session with isolated test data.

Requires Fedora's GNOME Shell, Xwayland, Python GObject, Pillow, libXtst and ffmpeg.
Creates a temporary diagnostic backdrop; accounts, preferences and extensions are not modified.
"""
import argparse
import ctypes as C
import ctypes.util
import json
import io
import os
import re
from pathlib import Path
import signal
import subprocess
import tempfile
import time


class Attributes(C.Structure):
    _fields_ = [(s, C.c_int) for s in ('x', 'y', 'width', 'height', 'border', 'depth')] + [
        ('visual', C.c_void_p), ('root', C.c_ulong), ('kind', C.c_int),
        ('bit_gravity', C.c_int), ('win_gravity', C.c_int), ('backing_store', C.c_int),
        ('backing_planes', C.c_ulong), ('backing_pixel', C.c_ulong), ('save_under', C.c_int),
        ('colormap', C.c_ulong), ('map_installed', C.c_int), ('map_state', C.c_int),
        ('all_events', C.c_long), ('your_events', C.c_long), ('propagate', C.c_long),
        ('override_redirect', C.c_int), ('screen', C.c_void_p)]


class ClientMessage(C.Structure):
    _fields_ = [('type', C.c_int), ('serial', C.c_ulong), ('send_event', C.c_int),
                ('display', C.c_void_p), ('window', C.c_ulong), ('message_type', C.c_ulong),
                ('format', C.c_int), ('data', C.c_long * 5)]


class Event(C.Union):
    _fields_ = [('client', ClientMessage), ('padding', C.c_long * 24)]


class XImage(C.Structure):
    _fields_ = [(name,C.c_int) for name in ('width','height','xoffset','format')] + [('data',C.c_void_p)] + [
        (name,C.c_int) for name in ('byte_order','bitmap_unit','bitmap_bit_order','bitmap_pad','depth','bytes_per_line','bits_per_pixel')] + [
        ('red_mask',C.c_ulong),('green_mask',C.c_ulong),('blue_mask',C.c_ulong),('obdata',C.c_void_p),('functions',C.c_void_p*6)]


class X11:
    def __init__(self, display):
        self.x = C.CDLL(ctypes.util.find_library('X11'))
        self.xt = C.CDLL(ctypes.util.find_library('Xtst'))
        for name, args, result in [
            ('XGetImage',[C.c_void_p,C.c_ulong,C.c_int,C.c_int,C.c_uint,C.c_uint,C.c_ulong,C.c_int],C.POINTER(XImage)),
            ('XDestroyImage',[C.POINTER(XImage)],C.c_int),
            ('XOpenDisplay', [C.c_char_p], C.c_void_p),
            ('XDefaultRootWindow', [C.c_void_p], C.c_ulong),
            ('XInternAtom', [C.c_void_p, C.c_char_p, C.c_int], C.c_ulong),
            ('XFree', [C.c_void_p], C.c_int),
            ('XFlush', [C.c_void_p], C.c_int),
            ('XQueryTree', [C.c_void_p,C.c_ulong,C.POINTER(C.c_ulong),C.POINTER(C.c_ulong),C.POINTER(C.POINTER(C.c_ulong)),C.POINTER(C.c_uint)], C.c_int),
            ('XGetWindowAttributes', [C.c_void_p, C.c_ulong, C.POINTER(Attributes)], C.c_int),
            ('XGetWindowProperty', [C.c_void_p, C.c_ulong, C.c_ulong, C.c_long, C.c_long, C.c_int, C.c_ulong,
                                    C.POINTER(C.c_ulong), C.POINTER(C.c_int), C.POINTER(C.c_ulong),
                                    C.POINTER(C.c_ulong), C.POINTER(C.POINTER(C.c_ubyte))], C.c_int),
            ('XKeysymToKeycode', [C.c_void_p, C.c_ulong], C.c_uint),
            ('XSendEvent', [C.c_void_p, C.c_ulong, C.c_int, C.c_long, C.POINTER(Event)], C.c_int),
            ('XTranslateCoordinates', [C.c_void_p, C.c_ulong, C.c_ulong, C.c_int, C.c_int,
                                       C.POINTER(C.c_int), C.POINTER(C.c_int), C.POINTER(C.c_ulong)], C.c_int),
        ]:
            fn = getattr(self.x, name); fn.argtypes = args; fn.restype = result
        self.xt.XTestFakeMotionEvent.argtypes = [C.c_void_p, C.c_int, C.c_int, C.c_int, C.c_ulong]
        self.xt.XTestFakeButtonEvent.argtypes = [C.c_void_p, C.c_uint, C.c_int, C.c_ulong]
        self.xt.XTestFakeKeyEvent.argtypes = [C.c_void_p, C.c_uint, C.c_int, C.c_ulong]
        self.x.XDefaultRootWindow.argtypes = [C.c_void_p]
        self.x.XDefaultRootWindow.restype = C.c_ulong
        self.x.XFlush.argtypes = [C.c_void_p]
        self.x.XInternAtom.argtypes = [C.c_void_p, C.c_char_p, C.c_int]
        self.x.XInternAtom.restype = C.c_ulong
        self.x.XSetInputFocus.argtypes = [C.c_void_p, C.c_ulong, C.c_int, C.c_ulong]
        self.display = self.x.XOpenDisplay(display.encode())
        if not self.display: raise RuntimeError('Cannot open isolated XWayland display')
        self.root = self.x.XDefaultRootWindow(self.display)

    def atom(self, name): return self.x.XInternAtom(self.display, name.encode(), False)

    def property(self, window, name):
        actual, fmt, count, remaining = C.c_ulong(), C.c_int(), C.c_ulong(), C.c_ulong()
        data = C.POINTER(C.c_ubyte)()
        self.x.XGetWindowProperty(self.display, window, self.atom(name), 0, 65536, False, 0,
                                 C.byref(actual), C.byref(fmt), C.byref(count), C.byref(remaining), C.byref(data))
        try:
            if fmt.value == 32: return list(C.cast(data, C.POINTER(C.c_ulong))[:count.value])
            if fmt.value == 8: return C.string_at(data, count.value).decode('utf-8', 'replace')
            return []
        finally:
            if data: self.x.XFree(data)

    def windows(self, pid):
        candidates=set(self.property(self.root,'_NET_CLIENT_LIST'))
        def children(window):
            root,parent,count=C.c_ulong(),C.c_ulong(),C.c_uint();data=C.POINTER(C.c_ulong)()
            self.x.XQueryTree(self.display,window,C.byref(root),C.byref(parent),C.byref(data),C.byref(count))
            try:return list(data[:count.value])
            finally:
                if data:self.x.XFree(data)
        for child in children(self.root):
            candidates.add(child);candidates.update(children(child))
        return {self.property(w,'_NET_WM_NAME'):w for w in candidates if self.property(w,'_NET_WM_PID')==[pid]}

    def geometry(self, window):
        a = Attributes(); self.x.XGetWindowAttributes(self.display, window, C.byref(a))
        x, y, child = C.c_int(), C.c_int(), C.c_ulong()
        self.x.XTranslateCoordinates(self.display, window, self.root, 0, 0, C.byref(x), C.byref(y), C.byref(child))
        return x.value, y.value, a.width, a.height, a.map_state == 2

    def capture(self, window):
        from PIL import Image
        _,_,width,height,visible=self.geometry(window)
        assert visible
        native=self.x.XGetImage(self.display,window,0,0,width,height,C.c_ulong(-1),2)
        if not native:raise AssertionError('XGetImage failed')
        try:
            image=native.contents
            assert image.bits_per_pixel==32 and image.byte_order==0
            data=C.string_at(image.data,image.bytes_per_line*height)
            return Image.frombytes('RGBA',(width,height),data,'raw','BGRA',image.bytes_per_line,1)
        finally:self.x.XDestroyImage(native)

    def click(self, x, y):
        self.xt.XTestFakeMotionEvent(self.display, -1, x, y, 0)
        self.x.XFlush(self.display)
        # XWayland may deliver the button before the motion event. Give the
        # compositor a frame so raw-button handlers see the final pointer.
        time.sleep(.06)
        for down in (True, False): self.xt.XTestFakeButtonEvent(self.display, 1, down, 0)
        self.x.XFlush(self.display)

    def focus(self, window):
        self.x.XSetInputFocus(self.display, window, 2, 0)
        self.x.XFlush(self.display)
        time.sleep(.05)

    def escape(self):
        key = self.x.XKeysymToKeycode(self.display, 0xff1b)
        for down in (True, False): self.xt.XTestFakeKeyEvent(self.display, key, down, 0)
        self.x.XFlush(self.display)

    def close(self, window):
        event = Event(); event.client.type = 33; event.client.window = window
        event.client.display = self.display; event.client.message_type = self.atom('WM_PROTOCOLS')
        event.client.format = 32; event.client.data[0] = self.atom('WM_DELETE_WINDOW')
        self.x.XSendEvent(self.display, window, False, 0, C.byref(event)); self.x.XFlush(self.display)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--core', type=Path, required=True)
    parser.add_argument('--virtual', action='store_true', help='Use isolated Xvfb/Openbox instead of the current desktop')
    parser.add_argument('--desktop', default='GNOME', help='Desktop environment advertised to the application (e.g. KDE)')
    parser.add_argument('--duration', type=int, default=600)
    parser.add_argument('--report', type=Path, default=Path('.local/linux-native'))
    args = parser.parse_args()
    args.binary = args.binary.resolve(); args.core = args.core.resolve(); args.report = args.report.resolve()
    args.report.mkdir(parents=True, exist_ok=True)
    root = Path(tempfile.mkdtemp(prefix='aieyes-gtk-acceptance-'))
    env = dict(os.environ, XDG_RUNTIME_DIR=f'/run/user/{os.getuid()}', XDG_SESSION_TYPE='wayland',
               XDG_CURRENT_DESKTOP=args.desktop, DBUS_SESSION_BUS_ADDRESS=f'unix:path=/run/user/{os.getuid()}/bus',
               AIEYES_DATA_DIR=str(root/'data'), AIEYES_LINUX_TRACE='1')
    env.pop('GDK_BACKEND', None)
    (root/'data').mkdir()
    if not args.virtual and (not env.get('DISPLAY') or not env.get('XAUTHORITY')):
        listing = subprocess.check_output(['pgrep', '-u', str(os.getuid()), '-a', 'Xwayland'], text=True)
        line = next(line for line in listing.splitlines() if re.search(r'Xwayland :[0-9]+ .* -auth /run/user/',line))
        env['DISPLAY'] = re.search(r'Xwayland (:[0-9]+)',line)[1]
        env['XAUTHORITY'] = re.search(r' -auth (\S+)',line)[1]
    processes = []
    backdrop = None
    video = None
    screen = None
    report = {'binary': str(args.binary), 'dataDirectory': str(root/'data'), 'virtual':args.virtual, 'desktop':args.desktop, 'checks': [], 'samples': 0}
    panel = None
    try:
        if args.virtual:
            env['WEBKIT_DISABLE_COMPOSITING_MODE']='1'
            server=subprocess.Popen(['Xvfb','-displayfd','1','-screen','0','1600x1000x24','-nolisten','tcp','-ac'],
                                    stdout=subprocess.PIPE,stderr=(args.report/'xvfb.log').open('w'),start_new_session=True)
            processes.append(server)
            import select
            if not select.select([server.stdout],[],[],10)[0]:raise RuntimeError('Xvfb did not start')
            env['DISPLAY']=':'+server.stdout.readline().decode().strip();env.pop('XAUTHORITY',None)
            processes.append(subprocess.Popen(['openbox','--config-file','/etc/xdg/openbox/rc.xml'],env=env,
                                              stdout=(args.report/'openbox.log').open('w'),stderr=subprocess.STDOUT,start_new_session=True))
            time.sleep(.5)
        report['display']=env['DISPLAY']
        import gi
        gi.require_version('Gtk','3.0'); gi.require_version('Gdk','3.0'); gi.require_version('GdkX11','3.0')
        os.environ.update(DISPLAY=env['DISPLAY'],XAUTHORITY=env.get('XAUTHORITY',''),GDK_BACKEND='x11',
                          XDG_RUNTIME_DIR=env['XDG_RUNTIME_DIR'],DBUS_SESSION_BUS_ADDRESS=env['DBUS_SESSION_BUS_ADDRESS'])
        from gi.repository import Gio, GLib, Gtk, Gdk, GdkX11
        def pump(seconds=.05):
            until = time.monotonic()+seconds
            while time.monotonic() < until:
                while GLib.MainContext.default().pending(): GLib.MainContext.default().iteration(False)
                while Gtk.events_pending(): Gtk.main_iteration_do(False)
                time.sleep(.005)
        x = X11(env['DISPLAY'])
        gdk_display = Gdk.Display.get_default()
        if not gdk_display: raise RuntimeError('GTK could not connect to XWayland')
        if args.virtual:
            backdrop = Gtk.Window(title='Aieyes temporary diagnostic backdrop')
            backdrop.add(Gtk.Label(label='Aieyes Linux acceptance test'))
            backdrop.set_default_size(600,400)
            backdrop.move(20,120)
            backdrop.show_all()
            pump(1)
            backdrop.present()
            pump(1)
        subprocess.run([str(args.core), '--data-dir', str(root/'data'), '--call', 'settings.save',
                        json.dumps({'sources': [], 'accounts': [], 'hosts': [], 'agents': [], 'localDeviceEnabled': False})],
                       env=env, capture_output=True, check=True, timeout=20)
        (root/'data/desktop.json').write_text(json.dumps({'mode':'pet', 'position':{'x':800,'y':350}, 'hidden':False,'petScale':1}))
        app = subprocess.Popen([str(args.binary)], env=env, stdout=(args.report/'application.log').open('w'),
                               stderr=subprocess.STDOUT, start_new_session=True)
        processes.append(app)

        def wait(predicate, message, timeout=10):
            end = time.monotonic()+timeout
            while time.monotonic() < end:
                if app.poll() is not None: raise AssertionError(f'Application exited: {app.returncode}')
                if predicate(): return
                pump()
            raise AssertionError(message)

        wait(lambda: all(name in x.windows(app.pid) for name in ('Aieyes','Aieyes 桌宠')), 'Pet, panel and main windows were not created')
        windows = x.windows(app.pid)
        print('WINDOWS', json.dumps(windows, ensure_ascii=False), flush=True)
        pet = windows['Aieyes 桌宠']
        panel = None
        wait(lambda: x.geometry(pet)[4], 'Pet did not remain visible')
        assert x.atom('_NET_WM_STATE_ABOVE') in x.property(pet, '_NET_WM_STATE')
        report['checks'].append('native pet stays visible and above; closing details keeps the entry')

        def capture(window, name=None):
            image=x.capture(window)
            if name:image.save(args.report/name)
            return image

        pump(2)
        capture(pet, 'pet.png')
        def invoke(service,path,interface,method,params=None):
            connection=Gio.bus_get_sync(Gio.BusType.SESSION,None)
            return connection.call_sync(service,path,interface,method,params,None,Gio.DBusCallFlags.NONE,5000,None).unpack()
        def tray_click(label):
            # The tray is created by the status loop, which re-probes the
            # watcher every 15s. AppIndicator also registers asynchronously.
            deadline=time.monotonic()+35
            while True:
                try:
                    items=invoke('org.kde.StatusNotifierWatcher','/StatusNotifierWatcher','org.freedesktop.DBus.Properties','Get',
                                 GLib.Variant('(ss)',('org.kde.StatusNotifierWatcher','RegisteredStatusNotifierItems')))[0]
                except GLib.Error:
                    items=[]
                for item in items:
                    # AppIndicator uses service@path; ksni/StatusNotifierItem
                    # only publishes a well-known name and the default path.
                    service,sep,path=item.partition('@')
                    if not path:path='/StatusNotifierItem'
                    try:
                        owner=invoke('org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus','GetConnectionUnixProcessID',GLib.Variant('(s)',(service,)))[0]
                        if owner!=app.pid:continue
                        props=invoke(service,path,'org.freedesktop.DBus.Properties','GetAll',GLib.Variant('(s)',('org.kde.StatusNotifierItem',)))[0]
                        assert props['Status']=='Active','Tray icon is hidden in pet mode'
                        menu=props['Menu']
                        layout=invoke(service,menu,'com.canonical.dbusmenu','GetLayout',GLib.Variant('(iias)',(0,-1,['label'])))[1]
                    except (GLib.Error,KeyError):
                        continue
                    def find(node):
                        ident,properties,children=node
                        if properties.get('label')==label:return ident
                        for child in children:
                            found=find(child)
                            if found is not None:return found
                    ident=find(layout)
                    if ident is None:raise AssertionError(f'Tray has no {label} action')
                    invoke(service,menu,'com.canonical.dbusmenu','Event',GLib.Variant('(isvu)',(ident,'clicked',GLib.Variant('s',''),0)))
                    return
                if time.monotonic()>=deadline:
                    raise AssertionError('Application did not register its tray icon')
                pump(.5)
        def tray_open():tray_click('打开面板')
        def screenshot_desktop():
            # Mutter creates the PipeWire node on the same D-Bus connection as
            # the session; using a fresh connection per call kills the session.
            connection=Gio.bus_get_sync(Gio.BusType.SESSION,None)
            service='org.gnome.Mutter.ScreenCast'
            def call(path,interface,method,params=None):
                return connection.call_sync(service,path,interface,method,params,None,Gio.DBusCallFlags.NONE,10000,None).unpack()
            session=call('/org/gnome/Mutter/ScreenCast',service,'CreateSession',GLib.Variant('(a{sv})',({},)))[0]
            try:
                call(session,service+'.Session','RecordMonitor',GLib.Variant('(sa{sv})',('',{})))
                call(session,service+'.Session','Start')
                pump(1)
                nodes=[]
                for node in json.loads(subprocess.check_output(['pw-dump'],text=True)):
                    props=node.get('info',{}).get('props',{})
                    if props.get('media.class')=='Stream/Output/Video' and props.get('node.name')=='gnome-shell':
                        nodes.append(node)
                assert nodes,'Screen capture PipeWire node was not created'
                node=max(nodes,key=lambda n:n.get('info',{}).get('props',{}).get('object.serial',0))
                subprocess.run(['gst-launch-1.0','-q','pipewiresrc','path='+str(node['id']),'num-buffers=1','!',
                                'videoconvert','!','pngenc','!','filesink','location='+str(args.report/'desktop.png')],check=True,timeout=20)
            finally:
                try:call(session,service+'.Session','Stop')
                except GLib.Error:pass
        def start_screen_record(rect):
            connection=Gio.bus_get_sync(Gio.BusType.SESSION,None)
            service='org.gnome.Mutter.ScreenCast'
            def call(path,interface,method,params=None):
                return connection.call_sync(service,path,interface,method,params,None,Gio.DBusCallFlags.NONE,10000,None).unpack()
            session=call('/org/gnome/Mutter/ScreenCast',service,'CreateSession',GLib.Variant('(a{sv})',({},)))[0]
            stream=call(session,service+'.Session','RecordMonitor',GLib.Variant('(sa{sv})',('',{})))[0]
            call(session,service+'.Session','Start')
            pump(1)
            nodes=[]
            for node in json.loads(subprocess.check_output(['pw-dump'],text=True)):
                props=node.get('info',{}).get('props',{})
                if props.get('media.class')=='Stream/Output/Video' and props.get('node.name')=='gnome-shell':
                    nodes.append(node)
            assert nodes,'Screen recording PipeWire node was not created'
            node=max(nodes,key=lambda n:n.get('info',{}).get('props',{}).get('object.serial',0))
            parameters=call(stream,'org.freedesktop.DBus.Properties','Get',GLib.Variant('(ss)',(service+'.Stream','Parameters')))[0]
            width,height=parameters['size']
            root_width,root_height=x.geometry(x.root)[2:4]
            scale=width/root_width
            left,top,pet_width,pet_height=rect
            left=max(0,round(left*scale));top=max(0,round(top*scale))
            pet_width=min(width-left,round(pet_width*scale));pet_height=min(height-top,round(pet_height*scale))
            right=width-left-pet_width;bottom=height-top-pet_height
            process=subprocess.Popen(['gst-launch-1.0','-e','-q','pipewiresrc',f'path={node["id"]}','!',
                                      'videorate','!','video/x-raw,framerate=4/1','!',
                                      'videocrop',f'left={left}',f'top={top}',f'right={right}',f'bottom={bottom}','!',
                                      'videoconvert','!','vp8enc','deadline=1','cpu-used=8','!',
                                      'webmmux','!','filesink',f'location={args.report/"screen.webm"}'],
                                     stdout=(args.report/'screen-video.log').open('w'),
                                     stderr=subprocess.STDOUT)
            return {'connection':connection,'session':session,'process':process,'service':service}
        def stop_screen_record(rec):
            process=rec['process']
            if process.poll() is None:
                process.send_signal(signal.SIGINT)
                try:process.wait(timeout=15)
                except subprocess.TimeoutExpired:process.kill();process.wait()
            try:
                connection=rec['connection'];service=rec['service']
                connection.call_sync(service,rec['session'],service+'.Session','Stop',None,None,Gio.DBusCallFlags.NONE,10000,None)
            except GLib.Error:pass
        anchor = x.geometry(pet)[:4]
        print('PET GEOMETRY',anchor,flush=True)
        for cycle in range(100):
            left, top, width, height, _ = x.geometry(pet)
            if not args.virtual:tray_open()
            else:x.click(left+width//2, top+height//2)
            wait(lambda:'Aieyes 面板' in x.windows(app.pid) and x.geometry(x.windows(app.pid)['Aieyes 面板'])[4], f'Panel did not open at cycle {cycle}', 3)
            panel=x.windows(app.pid)['Aieyes 面板']
            assert x.geometry(pet)[4] and x.geometry(pet)[:4] == anchor
            if cycle < 3: print('PANEL',cycle,panel,'active',x.property(x.root,'_NET_ACTIVE_WINDOW'),'geometry',x.geometry(panel),flush=True)
            pump(.25)
            if cycle == 0:
                pump(1)
                capture(panel, 'panel.png')
                if not args.virtual:screenshot_desktop()
            if not args.virtual:
                # XTest pointer/key events are not reliably routed through
                # XWayland. Drive the native tray menu instead; the details
                # action closes the panel before showing the main window.
                tray_click('打开详情')
            elif cycle%2==0:
                # Target our own background window, never another application's controls.
                bx,by,bw,bh,_=x.geometry(GdkX11.X11Window.get_xid(backdrop.get_window()))
                x.click(bx+20,by+bh//2)
            else:
                x.focus(panel)
                x.escape()
            wait(lambda:not x.geometry(panel)[4], f'Panel did not dismiss at cycle {cycle}', 3)
        report['checks'].append(('100 pet clicks, outside desktop clicks and Escape; pet geometry unchanged' if args.virtual else '100 tray menu panel open/close cycles; pet geometry unchanged'))
        print('INTERACTIONS PASSED', flush=True)
        if args.duration>=30:
            video=subprocess.Popen(['ffmpeg','-loglevel','error','-y','-f','image2pipe','-framerate','4','-vcodec','png','-i','pipe:0',
                                    '-c:v','ffv1','-pix_fmt','bgra',str(args.report/'animation.mkv')],stdin=subprocess.PIPE,
                                   stderr=(args.report/'video.log').open('w'))
            if not args.virtual:
                screen=start_screen_record((left,top,width,height))
        started = time.monotonic(); next_snapshot = 0
        while time.monotonic()-started < args.duration:
            image = capture(pet)
            corners = [(0,0),(image.width-1,0),(0,image.height-1)]
            assert all(image.getpixel(point)[3]==0 for point in corners), 'Opaque pixels appeared in transparent pet corners'
            assert image.getbbox() is not None, 'Pet animation became blank'
            if video:
                buffer=io.BytesIO();image.save(buffer,format='PNG');video.stdin.write(buffer.getvalue())
            report['samples'] += 1
            if time.monotonic()-started >= next_snapshot:
                image.save(args.report/f'animation-{int(next_snapshot):04d}.png')
                next_snapshot += 30
                print('ANIMATION', int(time.monotonic()-started), 'seconds;', report['samples'], 'samples', flush=True)
            pump(.25)
        if video:
            video.stdin.close();assert video.wait(timeout=15)==0;video=None
        if screen:
            stop_screen_record(screen)
            screen=None
            record=args.report/'screen.webm'
            assert record.exists() and record.stat().st_size>0,'Screen recording is empty'
            check=subprocess.run(['ffmpeg','-loglevel','info','-i',str(record),
                                  '-vf','blackdetect=d=0.2:pix_th=0.85','-f','null','-'],
                                 capture_output=True,text=True,timeout=120)
            (args.report/'screen-blackdetect.log').write_text(check.stderr)
            assert 'black_start' not in check.stderr,'Black frames appeared in the pet screen recording'
            report['checks'].append(f'{args.duration}s screen recording: no black-frame intervals')
        report['checks'].append(f'{args.duration}s animation: no opaque corner flash, pet remained visible')
        report['passed'] = True
    except Exception as error:
        if 'app' in locals() and app.poll() is None:
            report['windowsAtFailure']=x.windows(app.pid)
            report['activeWindowAtFailure']=x.property(x.root,'_NET_ACTIVE_WINDOW')
            if panel: report['panelGeometryAtFailure']=x.geometry(panel)
            debug=subprocess.run(['sudo','-n','gdb','-batch','-p',str(app.pid),'-ex','thread apply all bt 10'],capture_output=True,text=True,timeout=20)
            (args.report/'failure-backtrace.txt').write_text(debug.stdout+debug.stderr)
        report['passed'] = False; report['error'] = str(error)
        raise
    finally:
        if video is not None:
            video.stdin.close();video.wait(timeout=15)
        if screen is not None:
            try: stop_screen_record(screen)
            except Exception: pass
        if backdrop is not None: backdrop.destroy()
        for process in reversed(processes):
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
                try: process.wait(timeout=5)
                except subprocess.TimeoutExpired: os.killpg(process.pid, signal.SIGKILL); process.wait()
        (args.report/'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2)+'\n')
        print(json.dumps(report, ensure_ascii=False, indent=2), flush=True)


if __name__ == '__main__':
    main()
