"""GNOME input for native acceptance tests, scoped to a temporary D-Bus session.

XTest does not reliably reach XWayland windows. Mutter's RemoteDesktop session
routes the same pointer/key input through the compositor. No desktop preferences
or extensions are changed, and closing the session releases its virtual devices.
"""
import os
import time


class MutterInput:
    remote = 'org.gnome.Mutter.RemoteDesktop'
    cast = 'org.gnome.Mutter.ScreenCast'

    def __init__(self, Gio, GLib, pump, desktop_size, pointer=None):
        self.Gio, self.GLib, self.pump = Gio, GLib, pump
        self.pointer = pointer
        self.connection = Gio.DBusConnection.new_for_address_sync(
            os.environ['DBUS_SESSION_BUS_ADDRESS'],
            Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION,
            None, None)
        self.session = None
        self.pipeline = None
        self.node_id = None
        self.buttons = set()
        try:
            self.session = self.call(self.remote, '/org/gnome/Mutter/RemoteDesktop', self.remote, 'CreateSession')[0]
            session_id = self.property(self.remote, self.session, self.remote + '.Session', 'SessionId')
            self.cast_session = self.call(self.cast, '/org/gnome/Mutter/ScreenCast', self.cast, 'CreateSession',
                                          '(a{sv})', ({'remote-desktop-session-id': GLib.Variant('s', session_id)},))[0]
            self.stream = self.call(self.cast, self.cast_session, self.cast + '.Session', 'RecordMonitor',
                                    '(sa{sv})', ('', {}))[0]
            self.parameters = self.property(self.cast, self.stream, self.cast + '.Stream', 'Parameters')
            self.desktop_size = desktop_size
            self.connection.signal_subscribe(self.cast, self.cast + '.Stream', 'PipeWireStreamAdded',
                                              self.stream, None, Gio.DBusSignalFlags.NONE,
                                              self.stream_added)
            self.call(self.remote, self.session, self.remote + '.Session', 'Start')
            deadline = time.monotonic() + 10
            while self.node_id is None and time.monotonic() < deadline:
                pump(.05)
            if self.node_id is None:
                raise RuntimeError('Mutter did not publish this session\'s PipeWire node')
            import gi
            gi.require_version('Gst', '1.0')
            from gi.repository import Gst
            self.Gst = Gst
            Gst.init(None)
            self.pipeline = Gst.parse_launch(
                f'pipewiresrc path={self.node_id} ! videoconvert ! video/x-raw,format=RGBA ! '
                'appsink name=frames max-buffers=1 drop=true sync=false')
            self.frames = self.pipeline.get_by_name('frames')
            self.pipeline.set_state(Gst.State.PLAYING)
            self.video_size = self.capture().size
            pump(.2)
        except Exception:
            self.close()
            raise

    def stream_added(self, connection, sender, path, interface, signal, parameters):
        self.node_id = parameters.unpack()[0]

    def capture(self):
        from PIL import Image
        sample = self.frames.emit('try-pull-sample', 10 * self.Gst.SECOND)
        if sample is None:
            error = self.pipeline.get_bus().pop_filtered(self.Gst.MessageType.ERROR)
            raise RuntimeError(f'No frame from Mutter: {error.parse_error() if error else "timed out"}')
        info = sample.get_caps().get_structure(0)
        width, height = info.get_value('width'), info.get_value('height')
        buffer = sample.get_buffer()
        data = buffer.extract_dup(0, buffer.get_size())
        return Image.frombytes('RGBA', (width, height), data, 'raw', 'RGBA', len(data) // height, 1)

    def call(self, service, path, interface, method, signature=None, values=None):
        parameters = self.GLib.Variant(signature, values) if signature else None
        return self.connection.call_sync(service, path, interface, method, parameters, None,
                                         self.Gio.DBusCallFlags.NONE, 10000, None).unpack()

    def property(self, service, path, interface, name):
        return self.call(service, path, 'org.freedesktop.DBus.Properties', 'Get', '(ss)', (interface, name))[0]

    def motion(self, x, y):
        # Absolute input is relative to the negotiated stream, whose pixel size
        # may differ from Parameters.size under fractional desktop scaling.
        width, height = self.video_size
        root_width, root_height = self.desktop_size
        self.call(self.remote, self.session, self.remote + '.Session', 'NotifyPointerMotionAbsolute',
                  '(sdd)', (self.stream, x * width / root_width, y * height / root_height))
        self.pump(.08)
        # XWayland freezes its queryable pointer during a compositor-owned
        # window drag; it catches up when the button is released.
        if self.pointer and not self.buttons:
            deadline = time.monotonic() + 3
            while True:
                actual = self.pointer()
                if abs(actual[0]-x) <= 2 and abs(actual[1]-y) <= 2:
                    break
                if time.monotonic() >= deadline:
                    raise RuntimeError(f'Compositor pointer did not reach {(x, y)}: {actual}')
                self.pump(.05)

    def button(self, button, down):
        code = {1: 0x110, 2: 0x112, 3: 0x111}[button]
        self.call(self.remote, self.session, self.remote + '.Session', 'NotifyPointerButton', '(ib)', (code, down))
        if down:
            self.buttons.add(button)
        else:
            self.buttons.discard(button)
        self.pump(.05)

    def click(self, x, y, button=1):
        self.motion(x, y)
        self.button(button, True)
        self.button(button, False)

    def click_video(self, x, y, button=1):
        # GNOME Shell itself is a Wayland surface. Its pointer is not reflected
        # by XQueryPointer, so use the observed compositor image coordinates.
        self.call(self.remote, self.session, self.remote + '.Session', 'NotifyPointerMotionAbsolute',
                  '(sdd)', (self.stream, x, y))
        self.pump(.15)
        self.button(button, True)
        self.button(button, False)

    def key(self, keysym):
        for down in (True, False):
            self.call(self.remote, self.session, self.remote + '.Session', 'NotifyKeyboardKeysym', '(ub)', (keysym, down))
            self.pump(.05)

    def escape(self):
        self.key(0xff1b)

    def drag(self, start, end):
        self.motion(*start)
        self.button(1, True)
        try:
            for step in range(1, 13):
                self.motion(*(a + (b - a) * step / 12 for a, b in zip(start, end)))
        finally:
            self.button(1, False)

    def close(self):
        if self.session:
            try:
                for button in tuple(self.buttons):
                    self.button(button, False)
                self.call(self.remote, self.session, self.remote + '.Session', 'Stop')
            except Exception:
                pass
            self.session = None
        if self.pipeline is not None:
            self.pipeline.set_state(self.Gst.State.NULL)
            self.pipeline = None
        if self.connection and not self.connection.is_closed():
            self.connection.close_sync(None)
