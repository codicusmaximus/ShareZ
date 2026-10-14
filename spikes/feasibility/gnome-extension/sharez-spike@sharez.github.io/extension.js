// ShareZ spike helper: answers org.sharez.SpikeHelper.GetWindows with every window's frame
// rectangle, in stacking order (bottom to top), plus monitor geometry and scale.
// Mutter does not expose this to normal Wayland clients, which is why the spike needs it.

import Gio from 'gi://Gio';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const IFACE = `
<node>
  <interface name="org.sharez.SpikeHelper">
    <method name="GetWindows">
      <arg type="s" direction="out" name="json"/>
    </method>
  </interface>
</node>`;

const rect = r => [r.x, r.y, r.width, r.height];

export default class SharezSpikeExtension extends Extension {
    enable() {
        this._dbus = Gio.DBusExportedObject.wrapJSObject(IFACE, this);
        this._dbus.export(Gio.DBus.session, '/org/sharez/SpikeHelper');
        this._nameId = Gio.bus_own_name(Gio.BusType.SESSION, 'org.sharez.SpikeHelper',
            Gio.BusNameOwnerFlags.NONE, null, null, null);
    }

    disable() {
        Gio.bus_unown_name(this._nameId);
        this._dbus.unexport();
        this._dbus = null;
    }

    GetWindows() {
        const display = global.display;
        const windows = global.get_window_actors().map(a => a.meta_window);
        const stacked = display.sort_windows_by_stacking(windows);
        const out = stacked.map(w => ({
            title: w.get_title(),
            wm_class: w.get_wm_class(),
            pid: w.get_pid(),
            frame: rect(w.get_frame_rect()),
            buffer: rect(w.get_buffer_rect()),
            window_type: w.get_window_type(),
            client_type: w.get_client_type() === 0 ? 'wayland' : 'x11',
            minimized: w.minimized,
            workspace: w.get_workspace()?.index() ?? null,
        }));
        const monitors = [];
        for (let i = 0; i < display.get_n_monitors(); i++) {
            monitors.push({geometry: rect(display.get_monitor_geometry(i)), scale: display.get_monitor_scale(i)});
        }
        return JSON.stringify({windows: out, monitors});
    }
}
