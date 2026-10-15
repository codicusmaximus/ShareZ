// ShareZ Helper: answers GetWindows with the windows on the active workspace, topmost first.
// Mutter gives no window positions or stacking order to normal Wayland clients.

import Gio from 'gi://Gio';
import Meta from 'gi://Meta';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const BUS_NAME = 'io.github.codicusmaximus.ShareZ.ShellHelper';
const OBJECT_PATH = '/io/github/codicusmaximus/ShareZ/ShellHelper';
const IFACE = `
<node>
  <interface name="io.github.codicusmaximus.ShareZ.ShellHelper">
    <method name="GetWindows">
      <arg type="s" direction="out" name="json"/>
    </method>
  </interface>
</node>`;

const rect = r => [r.x, r.y, r.width, r.height];

function kind(w) {
    switch (w.get_window_type()) {
    case Meta.WindowType.NORMAL: return 'normal';
    case Meta.WindowType.DIALOG:
    case Meta.WindowType.MODAL_DIALOG: return 'dialog';
    case Meta.WindowType.DOCK: return 'dock';
    case Meta.WindowType.DESKTOP: return 'desktop';
    default: return 'other';
    }
}

export default class ShareZHelperExtension extends Extension {
    enable() {
        this._dbus = Gio.DBusExportedObject.wrapJSObject(IFACE, this);
        this._dbus.export(Gio.DBus.session, OBJECT_PATH);
        this._nameId = Gio.bus_own_name(Gio.BusType.SESSION, BUS_NAME,
            Gio.BusNameOwnerFlags.NONE, null, null, null);
    }

    disable() {
        Gio.bus_unown_name(this._nameId);
        this._dbus.unexport();
        this._dbus = null;
    }

    GetWindows() {
        const workspace = global.workspace_manager.get_active_workspace();
        const windows = global.get_window_actors()
            .map(a => a.meta_window)
            .filter(w => !w.minimized && w.located_on_workspace(workspace));
        const stacked = global.display.sort_windows_by_stacking(windows).reverse();
        return JSON.stringify(stacked.map(w => {
            const frame = w.get_frame_rect();
            return {
                title: w.get_title() ?? '',
                app_id: w.get_wm_class(),
                pid: w.get_pid(),
                kind: kind(w),
                frame: rect(frame),
                client: rect(w.frame_rect_to_client_rect(frame)),
            };
        }));
    }
}
