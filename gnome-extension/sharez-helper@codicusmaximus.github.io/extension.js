// ShareZ Helper: answers GetWindows with the windows on the active workspace, topmost first,
// places ShareZ's recording outline with PlaceWindow, and shows ShareZ's icon in the top bar while
// ShareZ runs. Mutter gives normal Wayland clients neither window positions nor stacking order,
// nor a way to position their own windows, and GNOME has no tray for them.

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import St from 'gi://St';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';

const BUS_NAME = 'io.github.codicusmaximus.ShareZ.ShellHelper';
const OBJECT_PATH = '/io/github/codicusmaximus/ShareZ/ShellHelper';
const APP_ID = 'io.github.codicusmaximus.ShareZ';
const APP_PATH = '/io/github/codicusmaximus/ShareZ';
const IFACE = `
<node>
  <interface name="io.github.codicusmaximus.ShareZ.ShellHelper">
    <method name="Features">
      <arg type="as" direction="out" name="features"/>
    </method>
    <method name="SetRecording">
      <arg type="b" direction="in" name="recording"/>
    </method>
    <method name="GetWindows">
      <arg type="s" direction="out" name="json"/>
    </method>
    <method name="PlaceWindow">
      <arg type="s" direction="in" name="title"/>
      <arg type="i" direction="in" name="x"/>
      <arg type="i" direction="in" name="y"/>
      <arg type="i" direction="in" name="width"/>
      <arg type="i" direction="in" name="height"/>
      <arg type="b" direction="out" name="placed"/>
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

// Runs one of ShareZ's app actions, as its own menus do.
function activate(action) {
    Gio.DBus.session.call(APP_ID, APP_PATH, 'org.gtk.Actions', 'Activate',
        new GLib.Variant('(sava{sv})', [action, [], {}]), null,
        Gio.DBusCallFlags.NONE, -1, null, null);
}

export default class ShareZHelperExtension extends Extension {
    enable() {
        this._recording = false;
        this._dbus = Gio.DBusExportedObject.wrapJSObject(IFACE, this);
        this._dbus.export(Gio.DBus.session, OBJECT_PATH);
        this._nameId = Gio.bus_own_name(Gio.BusType.SESSION, BUS_NAME,
            Gio.BusNameOwnerFlags.NONE, null, null, null);
        // The icon is there exactly while ShareZ runs.
        this._watchId = Gio.bus_watch_name(Gio.BusType.SESSION, APP_ID,
            Gio.BusNameWatcherFlags.NONE,
            () => this._showIndicator(),
            () => this._hideIndicator());
    }

    disable() {
        Gio.bus_unwatch_name(this._watchId);
        this._hideIndicator();
        Gio.bus_unown_name(this._nameId);
        this._dbus.unexport();
        this._dbus = null;
    }

    Features() {
        return ['windows', 'place', 'indicator'];
    }

    SetRecording(recording) {
        this._recording = recording;
        this._updateIndicator();
    }

    _showIndicator() {
        if (this._indicator)
            return;
        this._indicator = new PanelMenu.Button(0.0, 'ShareZ', false);
        this._icon = new St.Icon({style_class: 'system-status-icon'});
        this._indicator.add_child(this._icon);
        const menu = this._indicator.menu;
        const item = (label, action) => {
            const i = new PopupMenu.PopupMenuItem(label);
            i.connect('activate', () => activate(action));
            menu.addMenuItem(i);
            return i;
        };
        this._capture = item('Capture Region', 'capture');
        this._record = item('Record Region', 'record');
        this._discard = item('Discard Recording', 'discard-recording');
        menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());
        item('Open ShareZ', 'show');
        item('Quit', 'quit');
        Main.panel.addToStatusArea(this.uuid, this._indicator);
        this._updateIndicator();
    }

    _hideIndicator() {
        this._recording = false;
        this._indicator?.destroy();
        this._indicator = null;
    }

    _updateIndicator() {
        if (!this._indicator)
            return;
        const recording = this._recording;
        this._icon.icon_name = recording ? 'media-record-symbolic' : 'applets-screenshooter-symbolic';
        this._capture.visible = !recording;
        this._discard.visible = recording;
        this._record.label.text = recording ? 'Stop Recording' : 'Record Region';
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

    // Moves the named window (one of ShareZ's own) to an exact spot, keeps it above other windows
    // on every workspace, and hands focus back to whatever had it.
    PlaceWindow(title, x, y, width, height) {
        const windows = global.get_window_actors().map(a => a.meta_window);
        const win = windows.find(w => w.get_title() === title);
        if (!win)
            return false;
        const hadFocus = global.display.focus_window === win;
        win.move_resize_frame(false, x, y, width, height);
        win.make_above();
        win.stick();
        if (hadFocus) {
            const others = global.display.sort_windows_by_stacking(windows.filter(w =>
                w !== win && !w.minimized && w.get_window_type() === Meta.WindowType.NORMAL));
            others.at(-1)?.activate(global.get_current_time());
        }
        return true;
    }
}
