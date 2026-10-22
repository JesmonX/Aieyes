"""A StatusNotifier test host for an isolated dbus-run-session, never the user's bus."""
import os


class TestWatcher:
    name = 'org.kde.StatusNotifierWatcher'
    path = '/StatusNotifierWatcher'
    xml = '''<node><interface name="org.kde.StatusNotifierWatcher">
      <method name="RegisterStatusNotifierItem"><arg type="s" direction="in"/></method>
      <method name="RegisterStatusNotifierHost"><arg type="s" direction="in"/></method>
      <property name="RegisteredStatusNotifierItems" type="as" access="read"/>
      <property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
      <property name="ProtocolVersion" type="i" access="read"/>
      <signal name="StatusNotifierItemRegistered"><arg type="s"/></signal>
      <signal name="StatusNotifierItemUnregistered"><arg type="s"/></signal>
      <signal name="StatusNotifierHostRegistered"/>
      <signal name="StatusNotifierHostUnregistered"/>
    </interface></node>'''

    def __init__(self, Gio, GLib):
        self.Gio, self.GLib = Gio, GLib
        self.address = os.environ['DBUS_SESSION_BUS_ADDRESS']
        if f'/run/user/{os.getuid()}/bus' in self.address:
            raise RuntimeError('Test watcher requires a private dbus-run-session')
        self.connection = None
        self.start(False)

    def start(self, available=True):
        Gio, GLib = self.Gio, self.GLib
        self.available, self.items = available, []
        self.connection = Gio.DBusConnection.new_for_address_sync(
            self.address, Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT |
            Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION, None, None)
        self.registration = self.connection.register_object(
            self.path, Gio.DBusNodeInfo.new_for_xml(self.xml).interfaces[0], self.method, self.property, None)
        result = self.connection.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus',
            'org.freedesktop.DBus', 'RequestName', GLib.Variant('(su)', (self.name, 4)),
            None, Gio.DBusCallFlags.NONE, 5000, None).unpack()[0]
        if result != 1:
            self.close()
            raise RuntimeError('Private test bus already has a StatusNotifier watcher')

    def property(self, connection, sender, path, interface, name):
        values = {'RegisteredStatusNotifierItems':('as', self.items),
                  'IsStatusNotifierHostRegistered':('b', self.available), 'ProtocolVersion':('i', 0)}
        return self.GLib.Variant(*values[name])

    def method(self, connection, sender, path, interface, method, parameters, invocation):
        if method == 'RegisterStatusNotifierItem':
            item = parameters.unpack()[0]
            if item.startswith('/'): item = sender + '@' + item
            if item not in self.items:
                self.items.append(item)
                self.connection.emit_signal(None, self.path, self.name, 'StatusNotifierItemRegistered',
                                            self.GLib.Variant('(s)', (item,)))
        invocation.return_value(self.GLib.Variant('()', ()))

    def set_host(self, available):
        self.available = available
        self.connection.emit_signal(None, self.path, 'org.freedesktop.DBus.Properties', 'PropertiesChanged',
            self.GLib.Variant('(sa{sv}as)', (self.name,
                {'IsStatusNotifierHostRegistered':self.GLib.Variant('b', available)}, [])))
        self.connection.emit_signal(None, self.path, self.name,
            'StatusNotifierHostRegistered' if available else 'StatusNotifierHostUnregistered', None)

    def close(self):
        if self.connection and not self.connection.is_closed():
            self.connection.close_sync(None)
        self.connection = None
