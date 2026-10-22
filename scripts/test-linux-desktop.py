#!/usr/bin/env python3
"""Exercise real GTK/Tauri windows in a GNOME/XWayland session with isolated test data.

Requires Fedora's GNOME Shell, Xwayland, Python GObject, Pillow, libXtst and ffmpeg.
Creates a temporary diagnostic backdrop; accounts, preferences and extensions are not modified.
"""
import argparse
import ctypes as C
import ctypes.util
import hashlib
import json
import io
import os
import re
from pathlib import Path
import signal
import subprocess
import tempfile
import time

from linux_desktop_session import MutterInput


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
            ('XQueryPointer', [C.c_void_p,C.c_ulong,C.POINTER(C.c_ulong),C.POINTER(C.c_ulong),
                               C.POINTER(C.c_int),C.POINTER(C.c_int),C.POINTER(C.c_int),C.POINTER(C.c_int),C.POINTER(C.c_uint)], C.c_int),
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
        pids = {pid} if isinstance(pid, int) else set(pid)
        candidates=set(self.property(self.root,'_NET_CLIENT_LIST'))
        def children(window):
            root,parent,count=C.c_ulong(),C.c_ulong(),C.c_uint();data=C.POINTER(C.c_ulong)()
            self.x.XQueryTree(self.display,window,C.byref(root),C.byref(parent),C.byref(data),C.byref(count))
            try:return list(data[:count.value])
            finally:
                if data:self.x.XFree(data)
        for child in children(self.root):
            candidates.add(child);candidates.update(children(child))
        return {self.property(w,'_NET_WM_NAME'):w for w in candidates
                if any(p in pids for p in self.property(w,'_NET_WM_PID'))}

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

    def motion(self, x, y):
        self.xt.XTestFakeMotionEvent(self.display, -1, x, y, 0)
        self.x.XFlush(self.display)
        # XWayland may deliver the button before the motion event. Give the
        # compositor a frame so raw-button handlers see the final pointer.
        time.sleep(.06)

    def pointer(self):
        root, child = C.c_ulong(), C.c_ulong()
        rx, ry, wx, wy, mask = C.c_int(), C.c_int(), C.c_int(), C.c_int(), C.c_uint()
        self.x.XQueryPointer(self.display, self.root, C.byref(root), C.byref(child), C.byref(rx), C.byref(ry),
                             C.byref(wx), C.byref(wy), C.byref(mask))
        return rx.value, ry.value, mask.value

    def button(self, button, down):
        self.xt.XTestFakeButtonEvent(self.display, button, down, 0)
        self.x.XFlush(self.display)
        time.sleep(.05)

    def click(self, x, y, button=1):
        self.motion(x, y)
        for down in (True, False): self.button(button, down)

    def drag(self, start, end):
        self.motion(*start)
        self.button(1, True)
        try:
            for step in range(1, 13):
                self.motion(*(round(a + (b-a)*step/12) for a, b in zip(start, end)))
        finally:
            self.button(1, False)

    def focus(self, window):
        self.x.XSetInputFocus(self.display, window, 2, 0)
        self.x.XFlush(self.display)
        time.sleep(.05)

    def key(self, keysym):
        key = self.x.XKeysymToKeycode(self.display, keysym)
        for down in (True, False): self.xt.XTestFakeKeyEvent(self.display, key, down, 0)
        self.x.XFlush(self.display)

    def escape(self):
        self.key(0xff1b)

    def close(self, window):
        event = Event(); event.client.type = 33; event.client.window = window
        event.client.display = self.display; event.client.message_type = self.atom('WM_PROTOCOLS')
        event.client.format = 32; event.client.data[0] = self.atom('WM_DELETE_WINDOW')
        self.x.XSendEvent(self.display, window, False, 0, C.byref(event)); self.x.XFlush(self.display)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--core', type=Path, required=True)
    parser.add_argument('--package', type=Path, help='Installer whose installed binary/AppImage is being exercised')
    parser.add_argument('--launch-arg', action='append', default=[], help='Argument passed to the actual launcher, e.g. --launch-arg=--appimage-extract-and-run')
    parser.add_argument('--virtual', action='store_true', help='Use isolated Xvfb/Openbox instead of the current desktop')
    parser.add_argument('--desktop', default='GNOME', help='Desktop environment advertised to the application (e.g. KDE)')
    parser.add_argument('--duration', type=int, default=600)
    parser.add_argument('--cycles', type=int, default=100)
    parser.add_argument('--scenario', choices=('basic', 'menus', 'lifecycle', 'all'), default='all')
    parser.add_argument('--tray-cycle', action='store_true', help='Simulate host loss/recovery on a private D-Bus/Xvfb session')
    parser.add_argument('--shell-tray', action='store_true', help='Click the visible GNOME top-bar icon and capture its native menu')
    parser.add_argument('--language', choices=('zh-CN', 'en'), default='zh-CN')
    parser.add_argument('--theme', choices=('light', 'dark'), default='light')
    parser.add_argument('--backend', choices=('auto', 'x11'), default='auto')
    parser.add_argument('--report', type=Path, default=Path('.local/linux-native'))
    args = parser.parse_args()
    if args.tray_cycle and not args.virtual:
        parser.error('--tray-cycle requires --virtual and dbus-run-session')
    if args.shell_tray and (args.virtual or args.desktop != 'GNOME'):
        parser.error('--shell-tray requires the real GNOME session')
    if args.duration < 0 or args.cycles < 1:
        parser.error('duration must be nonnegative and cycles must be positive')
    args.binary = args.binary.resolve(); args.core = args.core.resolve(); args.report = args.report.resolve()
    args.report.mkdir(parents=True, exist_ok=True)
    root = Path(tempfile.mkdtemp(prefix='aieyes-gtk-acceptance-'))
    env = dict(os.environ, XDG_RUNTIME_DIR=f'/run/user/{os.getuid()}', XDG_SESSION_TYPE='wayland',
               XDG_CONFIG_HOME=str(root/'config'), XDG_CACHE_HOME=str(root/'cache'), XDG_DATA_HOME=str(root/'share'),
               XDG_CURRENT_DESKTOP=args.desktop,
               DBUS_SESSION_BUS_ADDRESS=os.environ.get('DBUS_SESSION_BUS_ADDRESS', f'unix:path=/run/user/{os.getuid()}/bus'),
               AIEYES_DATA_DIR=str(root/'data'), AIEYES_LINUX_TRACE='1')
    env.pop('GDK_BACKEND', None)
    if args.backend != 'auto': env['GDK_BACKEND'] = args.backend
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
    control = None
    watcher = None
    report = {'binary': str(args.binary), 'dataDirectory': str(root/'data'), 'virtual':args.virtual, 'desktop':args.desktop,
              'backend':args.backend, 'language':args.language, 'theme':args.theme, 'cycles':args.cycles,
              'scenario': args.scenario, 'durationSeconds':args.duration,
              'checks': [], 'samples': 0, 'animation': 'not-run', 'launchArguments':args.launch_arg}
    if args.package:
        report['package'] = {'path':str(args.package.resolve()), 'sha256':hashlib.sha256(args.package.read_bytes()).hexdigest()}
    labels = {'Aieyes 桌宠':'Aieyes pet', 'Aieyes 面板':'Aieyes panel', '打开面板':'Open panel',
              '打开详情':'Open details', '隐藏桌宠':'Hide pet', '显示桌宠':'Show pet',
              '设置…':'Settings…', '退出 Aieyes':'Quit Aieyes'}
    current_language = args.language
    def label(text): return labels.get(text, text) if current_language == 'en' else text
    pet_title, panel_title = label('Aieyes 桌宠'), label('Aieyes 面板')
    panel = None
    try:
        if args.virtual:
            env['WEBKIT_DISABLE_COMPOSITING_MODE']='1'
            server=subprocess.Popen(['Xvfb','-displayfd','1','-screen','0','1600x1000x24','-nolisten','tcp','-ac'],
                                    stdout=subprocess.PIPE,stderr=(args.report/'xvfb.log').open('w'),start_new_session=True)
            processes.append(server)
            import select
            if not select.select([server.stdout],[],[],10)[0]:raise RuntimeError('Xvfb did not start')
            display_number = server.stdout.readline().decode().strip()
            if not display_number.isdigit():
                raise RuntimeError(f'Xvfb failed to allocate a display; see {args.report / "xvfb.log"}')
            env['DISPLAY']=':'+display_number;env.pop('XAUTHORITY',None)
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
        backdrop = Gtk.Window(title='Aieyes temporary diagnostic backdrop')
        backdrop_keys = []
        backdrop.connect('key-press-event', lambda window, event: backdrop_keys.append(event.keyval) or False)
        backdrop.set_decorated(False)
        backdrop.add(Gtk.Label(label='Aieyes Linux acceptance test'))
        backdrop.set_default_size(600,400)
        backdrop.move(20,120)
        backdrop.show_all()
        pump(1)
        backdrop.present()
        pump(.3)
        control = x if args.virtual else MutterInput(Gio, GLib, pump, x.geometry(x.root)[2:4], pointer=x.pointer)
        report['input'] = 'XTest' if args.virtual else 'Mutter RemoteDesktop'
        if not args.virtual:
            report['compositorStream'] = control.parameters
            report['compositorVideoSize'] = control.video_size
        if args.tray_cycle:
            from linux_status_notifier_test import TestWatcher
            watcher = TestWatcher(Gio, GLib)
        def core_call(method, parameters=None):
            result = subprocess.run([str(args.core), '--data-dir', str(root/'data'), '--call', method,
                                     json.dumps(parameters or {})],
                                    env=env, capture_output=True, check=True, timeout=20)
            return json.loads(result.stdout)

        core_call('settings.save', {'sources': [], 'accounts': [], 'hosts': [], 'agents': [], 'localDeviceEnabled': False,
                                    'appearance':{'language':args.language, 'theme':args.theme}})
        (root/'data/desktop.json').write_text(json.dumps({'mode':'tray' if args.tray_cycle else 'pet', 'position':{'x':800,'y':350}, 'hidden':False,'petScale':1}))
        app = subprocess.Popen([str(args.binary), *args.launch_arg], env=env, stdout=(args.report/'application.log').open('w'),
                               stderr=subprocess.STDOUT, start_new_session=True)
        processes.append(app)
        app_pid = None

        def process_ids():
            if app_pid: return {app_pid} if Path(f'/proc/{app_pid}').exists() else set()
            # AppImage may fork its real application. Follow only this launch's
            # process group, never another running Aieyes instance.
            found = set()
            for path in Path('/proc').glob('[0-9]*/stat'):
                try:
                    fields = path.read_text().rpartition(') ')[2].split()
                    if int(fields[2]) == app.pid: found.add(int(path.parent.name))
                except (OSError, ValueError, IndexError): pass
            return found

        def windows(): return x.windows(process_ids())

        def wait(predicate, message, timeout=10):
            end = time.monotonic()+timeout
            while time.monotonic() < end:
                if app.poll() is not None and not process_ids(): raise AssertionError(f'Application exited: {app.returncode}')
                if predicate(): return
                pump()
            raise AssertionError(message)

        wait(lambda: all(name in windows() for name in ('Aieyes',pet_title)), 'Pet and main windows were not created', 30)
        initial_windows = windows()
        print('WINDOWS', json.dumps(initial_windows, ensure_ascii=False), flush=True)
        pet = initial_windows[pet_title]
        app_pid = x.property(pet, '_NET_WM_PID')[0]
        report['pid'] = app_pid
        report['windowClass'] = x.property(initial_windows['Aieyes'], 'WM_CLASS')
        panel = None
        wait(lambda: x.geometry(pet)[4], 'Pet did not remain visible')
        assert x.atom('_NET_WM_STATE_ABOVE') in x.property(pet, '_NET_WM_STATE')
        wait(lambda: x.geometry(initial_windows['Aieyes'])[4],
             'Initial main window did not map; verify the desktop native decoration service', 15)
        report['mainWindow'] = {'geometry':x.geometry(initial_windows['Aieyes']),
                                'frameExtents':x.property(initial_windows['Aieyes'], '_NET_FRAME_EXTENTS')}
        pump(1)
        x.capture(initial_windows['Aieyes']).save(args.report/'initial-main.png')
        if not args.virtual: control.capture().save(args.report/'initial-desktop.png')
        x.close(initial_windows['Aieyes'])
        wait(lambda:not x.geometry(initial_windows['Aieyes'])[4], 'Closing details did not hide the main window')
        assert x.geometry(pet)[4], 'Closing details hid the pet'
        report['checks'].append('native pet stays visible and above; closing details keeps the entry')

        def capture(window, name=None):
            if window == panel:
                wait(lambda: max(high for low,high in x.capture(window).getextrema()[:3]) > 64,
                     'Mapped panel did not paint its contents', 5)
            image=x.capture(window)
            if name:image.save(args.report/name)
            return image

        pump(2)
        capture(pet, 'pet.png')
        def invoke(service,path,interface,method,params=None):
            connection=Gio.bus_get_sync(Gio.BusType.SESSION,None)
            result = {}
            def finished(connection, task, unused):
                try: result['reply'] = connection.call_finish(task).unpack()
                except GLib.Error as error: result['error'] = error
            connection.call(service,path,interface,method,params,None,Gio.DBusCallFlags.NONE,5000,None,finished,None)
            while not result: pump(.01)
            if 'error' in result: raise result['error']
            return result['reply']
        def tray_click(label, activate=True):
            label = labels.get(label, label) if current_language == 'en' else label
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
                        if owner!=app_pid:continue
                        props=invoke(service,path,'org.freedesktop.DBus.Properties','GetAll',GLib.Variant('(s)',('org.kde.StatusNotifierItem',)))[0]
                        assert props['Status']=='Active','Tray icon is hidden in pet mode'
                        pixmaps = props.get('IconPixmap', [])
                        assert pixmaps and all(w > 0 and h > 0 and len(data) == w*h*4 for w,h,data in pixmaps), 'Invalid tray icon pixels'
                        if 'trayIcon' not in report:
                            w,h,data = pixmaps[0]
                            report['trayIcon'] = {'width':w, 'height':h, 'sha256':hashlib.sha256(bytes(data)).hexdigest()}
                            from PIL import Image
                            Image.frombytes('RGBA', (w,h), bytes(data), 'raw', 'ARGB').save(args.report/'tray-icon.png')
                        menu=props['Menu']
                        layout=invoke(service,menu,'com.canonical.dbusmenu','GetLayout',GLib.Variant('(iias)',(0,-1,['label','enabled'])))[1]
                    except (GLib.Error,KeyError):
                        continue
                    def find(node):
                        ident,properties,children=node
                        if properties.get('label')==label:
                            assert properties.get('enabled', True), f'Tray action {label} is disabled'
                            return ident
                        for child in children:
                            found=find(child)
                            if found is not None:return found
                    ident=find(layout)
                    if ident is None:raise AssertionError(f'Tray has no {label} action')
                    if activate:
                        invoke(service,menu,'com.canonical.dbusmenu','Event',GLib.Variant('(isvu)',(ident,'clicked',GLib.Variant('s',''),0)))
                    return
                if time.monotonic()>=deadline:
                    raise AssertionError('Application did not register its tray icon')
                pump(.5)
        def tray_open():tray_click('打开面板')
        def screenshot_desktop(name='desktop.png'):
            control.capture().save(args.report/name)

        def start_screen_record(rect):
            process = subprocess.Popen(
                ['ffmpeg', '-loglevel', 'error', '-y', '-f', 'image2pipe', '-framerate', '4',
                 '-vcodec', 'png', '-probesize', '32', '-analyzeduration', '0',
                 '-use_wallclock_as_timestamps', '1', '-i', 'pipe:0',
                 '-vsync', 'vfr', '-c:v', 'libvpx', '-deadline', 'realtime',
                 '-cpu-used', '8', str(args.report/'screen.webm')], stdin=subprocess.PIPE,
                stderr=(args.report/'screen-video.log').open('w'))
            return {'process': process, 'rect': rect}

        def screen_frame(rec):
            image = control.capture()
            root_width, root_height = x.geometry(x.root)[2:4]
            left, top, width, height = rec['rect']
            box = (round(left*image.width/root_width), round(top*image.height/root_height),
                   round((left+width)*image.width/root_width), round((top+height)*image.height/root_height))
            frame = image.crop(box).convert('RGB')
            buffer = io.BytesIO(); frame.save(buffer, format='PNG')
            rec['process'].stdin.write(buffer.getvalue())

        def stop_screen_record(rec):
            process = rec['process']
            process.stdin.close()
            try:
                assert process.wait(timeout=20) == 0, 'Compositor recording failed'
            except subprocess.TimeoutExpired:
                process.kill(); process.wait()
                raise
        anchor = x.geometry(pet)[:4]
        print('PET GEOMETRY',anchor,flush=True)
        for cycle in range(args.cycles):
            left, top, width, height, _ = x.geometry(pet)
            if not args.virtual and cycle % 2:tray_open()
            else:control.click(left+width//2, top+height//2)
            wait(lambda:panel_title in windows() and x.geometry(windows()[panel_title])[4], f'Panel did not open at cycle {cycle}', 5)
            panel=windows()[panel_title]
            assert x.geometry(pet)[4] and x.geometry(pet)[:4] == anchor
            if cycle < 3: print('PANEL',cycle,panel,'active',x.property(x.root,'_NET_ACTIVE_WINDOW'),'geometry',x.geometry(panel),flush=True)
            pump(.25)
            if cycle == 0:
                pump(1)
                capture(panel, 'panel.png')
                if not args.virtual:screenshot_desktop()
            if cycle%3==0:
                control.click(left+width//2, top+height//2)
            elif cycle%3==1:
                # Target our own background window, never another application's controls.
                bx,by,bw,bh,_=x.geometry(GdkX11.X11Window.get_xid(backdrop.get_window()))
                control.click(bx+20,by+bh//2)
            else:
                control.escape()
            wait(lambda:not x.geometry(panel)[4], f'Panel did not dismiss at cycle {cycle}', 3)
        entry_points = 'pet' if args.virtual else 'pet/tray'
        report['checks'].append(f'{args.cycles} real {entry_points} opens and pet/outside/Escape closes; pet geometry unchanged')
        print('INTERACTIONS PASSED', flush=True)
        def open_panel():
            nonlocal panel
            left, top, width, height, _ = x.geometry(pet)
            control.click(left+width//2, top+height//2)
            wait(lambda:panel_title in windows() and x.geometry(windows()[panel_title])[4], 'Panel did not reopen')
            panel = windows()[panel_title]
            pump(.4)

        def panel_click(px, py):
            left, top, width, height, _ = x.geometry(panel)
            # Panel controls use the shared 450 px layout. Negative y is an
            # offset from its bottom, also valid when the work area clamps it.
            scale = width / 450
            control.click(round(left+px*scale), round(top+(height+py*scale if py < 0 else py*scale)))
            pump(.3)

        def close_panel():
            # Let the compositor finish mapping/activating a tray-opened window
            # before pressing a control; X11 map state precedes its first frame.
            pump(.4)
            panel_click(377, 25)
            wait(lambda:not x.geometry(panel)[4], 'Panel close button did not work')

        def preferences():
            return json.loads((root/'data/desktop.json').read_text())

        def restart_application():
            nonlocal app, app_pid, pet, panel, main_window
            previous_pid = app_pid
            tray_click('退出 Aieyes')
            deadline = time.monotonic() + 15
            while Path(f'/proc/{previous_pid}').exists() and time.monotonic() < deadline:
                app.poll()  # Reap a directly launched executable.
                pump(.05)
            assert not Path(f'/proc/{previous_pid}').exists(), 'Tray Quit did not exit the application'
            app.wait(timeout=5)
            app_pid = None
            index = len(report.get('restarts', [])) + 1
            app = subprocess.Popen([str(args.binary), *args.launch_arg], env=env,
                                   stdout=(args.report/f'application-restart-{index}.log').open('w'),
                                   stderr=subprocess.STDOUT, start_new_session=True)
            processes.append(app)
            wait(lambda: all(name in windows() for name in ('Aieyes', pet_title)), 'Restart did not create windows', 35)
            pet = windows()[pet_title]
            app_pid = x.property(pet, '_NET_WM_PID')[0]
            main_window = windows()['Aieyes']
            wait(lambda:x.geometry(main_window)[4], 'Restart did not show the native main window')
            pump(1)
            x.close(main_window)
            wait(lambda:not x.geometry(main_window)[4], 'Restarted main window did not close')
            panel = windows().get(panel_title)
            report.setdefault('restarts', []).append({'pid':app_pid, 'preferences':preferences()})

        if args.scenario in ('menus', 'all'):
            for name, point in (('refresh', (415, 25)), ('more', (418, -19))):
                open_panel()
                before = capture(panel)
                panel_click(*point)
                after = capture(panel, f'{name}-menu.png')
                from PIL import ImageChops
                assert ImageChops.difference(before.convert('RGB'), after.convert('RGB')).getbbox(), f'{name} menu did not open'
                control.escape(); pump(.4)
                assert x.geometry(panel)[4], f'Esc closed the panel before its {name} menu'
                capture(panel, f'{name}-menu-dismissed.png')
                control.escape()
                wait(lambda:not x.geometry(panel)[4], 'Second Esc did not close panel')
                report['checks'].append(f'Esc closes {name} menu before panel')
            open_panel()
            panel_click(418, -19)
            panel_click(320, -90)
            capture(panel, 'pinned-menu.png')
            panel_click(95, 25)
            bx,by,bw,bh,_ = x.geometry(GdkX11.X11Window.get_xid(backdrop.get_window()))
            control.click(bx+20,by+bh//2); pump(.4)
            wait(backdrop.is_active, 'Outside click did not focus the diagnostic backdrop')
            assert x.geometry(panel)[4], 'Pinned panel closed on outside click'
            previous_escapes = backdrop_keys.count(0xff1b)
            control.escape()
            wait(lambda: backdrop_keys.count(0xff1b) > previous_escapes,
                 'Escape was not delivered to the focused diagnostic backdrop')
            pump(.3)
            assert x.geometry(panel)[4], 'Esc in another window closed the pinned panel'
            panel_click(95, 25)
            control.escape()
            wait(lambda:not x.geometry(panel)[4], 'Esc did not explicitly close the pinned panel')
            report['checks'].append('pin prevents outside dismissal; focused Esc still closes; external Esc leaves it open')
            open_panel()
            panel_click(418, -19); panel_click(320, -90)
            panel_click(95, 25)
            panel_click(418, -19); panel_click(320, -150)
            capture(panel, 'accounts-dialog.png')
            control.escape(); pump(.4)
            assert x.geometry(panel)[4], 'Esc closed panel before its accounts dialog'
            control.escape()
            wait(lambda:not x.geometry(panel)[4], 'Esc did not close panel after dialog')
            report['checks'].append('accounts dialog consumes Esc before panel')

        if args.scenario in ('lifecycle', 'all') and not args.virtual:
            open_panel()
            tray_click('隐藏桌宠')
            wait(lambda:not x.geometry(pet)[4] and not x.geometry(panel)[4], 'Hide pet left a pet or panel window visible')
            assert json.loads((root/'data/desktop.json').read_text())['hidden'] is True
            # Cover both per-second status updates and the 15-second host probe.
            pump(16)
            tray_click('显示桌宠', activate=False)
            assert not x.geometry(pet)[4], 'Background refresh discarded the hidden preference'
            report['checks'].append('hidden pet keeps its Show pet action after background status and tray-host refresh')
            tray_open()
            wait(lambda:x.geometry(panel)[4], 'Tray could not open panel while pet was hidden')
            assert not x.geometry(pet)[4], 'Opening panel changed hidden preference'
            capture(panel, 'panel-with-hidden-pet.png')
            close_panel()
            tray_click('显示桌宠')
            wait(lambda:x.geometry(pet)[4], 'Tray did not restore pet')
            assert json.loads((root/'data/desktop.json').read_text())['hidden'] is False
            report['checks'].append('tray remains active through hide/restore and opens panel with hidden pet')
            open_panel()
            previous = x.geometry(pet)[:4]
            previous_panel = x.geometry(panel)[:2]
            left,top,width,height = previous
            control.drag((left+width//2,top+height//2), (left+width//2+180,top+height//2+80))
            wait(lambda:x.geometry(pet)[:2] != previous[:2], 'Native pet did not move after drag')
            pump(.5)
            assert x.geometry(panel)[4], 'Dragging the pet dismissed its panel'
            assert x.geometry(panel)[:2] != previous_panel, 'Panel did not follow dragged pet'
            capture(pet, 'pet-after-drag.png')
            screenshot_desktop('desktop-after-drag.png')
            close_panel()
            report['checks'].append('pet drag retains panel coexistence and reanchors it')
            tray_click('打开详情')
            main_window = windows()['Aieyes']
            wait(lambda:x.geometry(main_window)[4], 'Tray did not reopen main window')
            pump(.7); capture(main_window, 'details.png')
            x.close(main_window)
            wait(lambda:not x.geometry(main_window)[4], 'Details close did not hide main window')
            assert x.geometry(pet)[4]
            report['checks'].append('tray reopens details; native window closes without quitting pet')
            left,top,width,height,_ = x.geometry(pet)
            control.click(left+width//2,top+height//2,3)
            pump(.2)
            control.key(0xff50)  # Home: first item in the native pet menu.
            control.key(0xff54); control.key(0xff54); control.key(0xff0d)
            wait(lambda:x.geometry(main_window)[4], 'Pet settings menu did not open main window')
            pump(1)
            capture(main_window, 'desktop-settings.png')
            def main_click(px, py):
                left,top,width,height,_ = x.geometry(main_window)
                scale = width / 1120
                control.click(round(left+px*scale), round(top+py*scale))
                pump(.3)

            # This exercises the actual settings control. It also catches a
            # native menu opening the wrong settings category.
            main_click(750, 180)
            control.key(0xff57); control.key(0xff0d)  # End, Enter: system tray.
            wait(lambda:preferences()['mode'] == 'tray' and not x.geometry(pet)[4],
                 'Desktop settings did not switch from pet to tray')
            capture(main_window, 'settings-tray-mode.png')
            x.close(main_window)
            wait(lambda:not x.geometry(main_window)[4], 'Closing pet settings failed')
            tray_open()
            wait(lambda:panel_title in windows() and x.geometry(windows()[panel_title])[4], 'Tray mode did not open panel')
            panel = windows()[panel_title]
            close_panel()
            saved_position = preferences()['position']
            restart_application()
            wait(lambda:not x.geometry(pet)[4], 'Tray mode was lost after restart', 35)
            assert preferences()['mode'] == 'tray'
            tray_click('显示桌宠')
            wait(lambda:x.geometry(pet)[4], 'Tray Show pet did not restore pet mode')
            assert preferences()['mode'] == 'pet'
            assert preferences()['position'] == saved_position
            assert x.geometry(pet)[:2] == (saved_position['x'], saved_position['y'])
            tray_click('隐藏桌宠')
            wait(lambda:not x.geometry(pet)[4], 'Hide failed before restart')
            restart_application()
            wait(lambda:not x.geometry(pet)[4], 'Hidden preference was lost after restart', 35)
            assert preferences()['hidden'] is True and preferences()['mode'] == 'pet'
            tray_open()
            wait(lambda:panel_title in windows() and x.geometry(windows()[panel_title])[4], 'Restarted hidden pet lost panel access')
            panel = windows()[panel_title]
            close_panel()
            tray_click('显示桌宠')
            wait(lambda:x.geometry(pet)[4], 'Hidden pet did not restore after restart')
            assert x.geometry(pet)[:2] == (saved_position['x'], saved_position['y'])
            report['checks'].append('pet context menu opens desktop settings; real mode selector switches to tray; tray returns to pet')
            report['checks'].append('graceful Quit/restart preserves tray mode, hidden preference and dragged position')

            open_panel()
            before_theme = capture(panel).convert('RGB').getpixel((40, 110))
            changed_theme = 'dark' if args.theme == 'light' else 'light'
            panel_click(274, -19)
            wait(lambda:core_call('settings.get')['appearance']['theme'] == changed_theme, 'Panel theme change did not persist')
            pump(.5)
            themed = capture(panel, f'panel-{changed_theme}.png').convert('RGB').getpixel((40, 110))
            assert sum(abs(a-b) for a,b in zip(before_theme, themed)) > 100, 'Panel did not repaint its theme'
            panel_click(225, -19)
            current_language = 'en' if args.language == 'zh-CN' else 'zh-CN'
            pet_title, panel_title = label('Aieyes 桌宠'), label('Aieyes 面板')
            wait(lambda:core_call('settings.get')['appearance']['language'] == current_language,
                 'Panel language change did not persist')
            wait(lambda:pet_title in windows() and panel_title in windows(), 'Native window titles did not change language')
            tray_click('隐藏桌宠', activate=False)
            capture(panel, f'panel-{current_language}-{changed_theme}.png')
            tray_click('打开详情')
            main_window = windows()['Aieyes']
            wait(lambda:x.geometry(main_window)[4], 'Details did not reopen after appearance changes')
            pump(.7)
            capture(main_window, f'details-{current_language}-{changed_theme}.png')
            x.close(main_window)
            wait(lambda:not x.geometry(main_window)[4], 'Details did not close after appearance changes')
            open_panel()
            panel_click(225, -19)
            current_language = args.language
            pet_title, panel_title = label('Aieyes 桌宠'), label('Aieyes 面板')
            wait(lambda:pet_title in windows() and panel_title in windows(), 'Original language did not restore')
            panel_click(274, -19)
            wait(lambda:core_call('settings.get')['appearance']['theme'] == args.theme, 'Original theme did not restore')
            close_panel()
            report['checks'].append('live theme/language changes persist through core IPC and reach panel, details, native titles and tray menu')
        if watcher:
            assert json.loads((root/'data/desktop.json').read_text())['mode'] == 'tray'
            watcher.set_host(True)
            wait(lambda:not x.geometry(pet)[4], 'Tray preference did not recover when a host appeared', 40)
            tray_open()
            wait(lambda:panel_title in windows() and x.geometry(windows()[panel_title])[4], 'Tray-only mode could not open panel')
            panel = windows()[panel_title]
            close_panel()
            tray_click('显示桌宠')
            wait(lambda:x.geometry(pet)[4], 'Tray-only mode did not switch back to pet')
            tray_click('隐藏桌宠')
            wait(lambda:not x.geometry(pet)[4], 'Hide failed before tray host loss')
            watcher.set_host(False)
            wait(lambda:x.geometry(pet)[4], 'Host loss stranded the hidden pet', 40)
            assert json.loads((root/'data/desktop.json').read_text())['hidden'] is True
            watcher.close(); pump(1); watcher.start()
            wait(lambda:not x.geometry(pet)[4], 'Watcher recovery did not restore hidden preference', 40)
            tray_click('显示桌宠')
            wait(lambda:x.geometry(pet)[4], 'Recreated watcher did not expose Show pet')
            report['checks'].append('private D-Bus: absent host fallback, tray mode recovery, hide, host loss and watcher restart')
        if args.shell_tray:
            # Locate the brand's cyan eye in the observed top bar. Fail rather
            # than click an ambiguous icon from another application.
            tray_click('隐藏桌宠', activate=False)
            before = control.capture().convert('RGB')
            pixels = before.load()
            def brand_pixel(rgb):
                r,g,b = rgb
                return b > 140 and g > 95 and r < 180 and b > r+12 and g > r+10
            mask = {(x,y) for x in range(before.width//2, before.width) for y in range(40)
                    if brand_pixel(pixels[x,y])}
            boxes = []
            while mask:
                start = mask.pop(); stack = [start]; group = [start]
                while stack:
                    x0,y0 = stack.pop()
                    for dx in (-1,0,1):
                        for dy in (-1,0,1):
                            point = (x0+dx, y0+dy)
                            if point in mask:
                                mask.remove(point); stack.append(point); group.append(point)
                xs,ys = zip(*group)
                box = (min(xs),min(ys),max(xs)+1,max(ys)+1)
                width,height = box[2]-box[0],box[3]-box[1]
                if 12 <= width <= 40 and 4 <= height <= 25 and 1.3 <= width/height <= 3.5:
                    boxes.append(box)
            assert len(boxes) == 1, f'GNOME tray eye could not be uniquely identified: {boxes}'
            left,top,right,bottom = boxes[0]
            control.click_video((left+right)/2, (top+bottom)/2)
            pump(.6)
            menu = control.capture().convert('RGB')
            region = (max(0,left-350),40,min(before.width,right+350),600)
            from PIL import ImageChops, ImageStat
            difference = ImageChops.difference(before.crop(region), menu.crop(region))
            bounds = difference.getbbox()
            assert bounds and bounds[2]-bounds[0] > 100 and bounds[3]-bounds[1] > 100, 'GNOME tray menu did not draw'
            opened = sum(ImageStat.Stat(difference).mean)
            menu.crop((region[0],0,region[2],region[3])).save(args.report/'shell-tray-menu.png')
            control.escape(); pump(.5)
            closed = ImageChops.difference(before.crop(region), control.capture().convert('RGB').crop(region))
            assert sum(ImageStat.Stat(closed).mean) < opened/2, 'GNOME tray menu did not close with Escape'
            report['shellTrayIconVideoBox'] = boxes[0]
            report['checks'].append('visible GNOME tray icon opens a rendered native menu; Escape closes it')
        if args.duration>=30:
            video=subprocess.Popen(['ffmpeg','-loglevel','error','-y','-f','image2pipe','-framerate','4','-vcodec','png',
                                    '-probesize','32','-analyzeduration','0','-use_wallclock_as_timestamps','1',
                                    '-i','pipe:0','-vsync','vfr',
                                    '-c:v','ffv1','-pix_fmt','bgra',str(args.report/'animation.mkv')],stdin=subprocess.PIPE,
                                   stderr=(args.report/'video.log').open('w'))
            if not args.virtual:
                screen=start_screen_record(x.geometry(pet)[:4])
        started = time.monotonic(); next_snapshot = 0
        animation_frames = set()
        while time.monotonic()-started < args.duration:
            image = capture(pet)
            corners = [(0,0),(image.width-1,0),(0,image.height-1),(image.width-1,image.height-1)]
            assert all(image.getpixel(point)[3]==0 for point in corners), 'Opaque pixels appeared in transparent pet corners'
            assert image.getbbox() is not None, 'Pet animation became blank'
            animation_frames.add(hashlib.sha256(image.tobytes()).digest())
            if screen: screen_frame(screen)
            if video:
                buffer=io.BytesIO();image.save(buffer,format='PNG');video.stdin.write(buffer.getvalue())
            report['samples'] += 1
            if time.monotonic()-started >= next_snapshot:
                image.save(args.report/f'animation-{int(next_snapshot):04d}.png')
                next_snapshot += 30
                print('ANIMATION', int(time.monotonic()-started), 'seconds;', report['samples'], 'samples', flush=True)
            pump(.25)
        report['animationWallSeconds'] = round(time.monotonic()-started, 3)
        report['uniqueAnimationFrames'] = len(animation_frames)
        if args.duration >= 5:
            assert len(animation_frames) > 1, 'Pet animation stayed frozen throughout the observation'
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
        for name in ('animation.mkv', 'screen.webm'):
            if args.duration >= 30 and (args.report/name).exists():
                duration = subprocess.check_output(['ffprobe', '-v', 'error', '-show_entries', 'format=duration',
                                                    '-of', 'json', str(args.report/name)], text=True, timeout=15)
                report.setdefault('videoDurationSeconds', {})[name] = float(json.loads(duration)['format']['duration'])
        if report['samples']:
            report['animation'] = 'passed'
            report['checks'].append(f'{args.duration}s animation: no opaque corner flash, pet remained visible')
        report['passed'] = True
    except Exception as error:
        if 'app' in locals() and app.poll() is None:
            report['windowsAtFailure']=windows()
            report['windowStatesAtFailure'] = {str(w):{'geometry':x.geometry(w), 'state':x.property(w,'WM_STATE'),
                                                       'desktop':x.property(w,'_NET_WM_DESKTOP'), 'netState':x.property(w,'_NET_WM_STATE')}
                                               for w in windows().values()}
            report['activeWindowAtFailure']=x.property(x.root,'_NET_ACTIVE_WINDOW')
            report['pointerAtFailure']=x.pointer()
            if panel: report['panelGeometryAtFailure']=x.geometry(panel)
            if isinstance(control, MutterInput):
                try: control.capture().save(args.report/'desktop-at-failure.png')
                except Exception: pass
        report['passed'] = False; report['error'] = str(error)
        raise
    finally:
        if video is not None:
            video.stdin.close();video.wait(timeout=15)
        if screen is not None:
            try: stop_screen_record(screen)
            except Exception: pass
        if isinstance(control, MutterInput): control.close()
        if watcher is not None: watcher.close()
        if backdrop is not None: backdrop.destroy()
        for process in reversed(processes):
            # AppImage's launcher can exit before its child. Always clean up
            # the isolated process group, including already-reaped launchers.
            try: os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError: pass
            try: process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL); process.wait()
        (args.report/'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2)+'\n')
        print(json.dumps(report, ensure_ascii=False, indent=2), flush=True)


if __name__ == '__main__':
    main()
