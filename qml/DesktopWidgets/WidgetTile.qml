import QtQuick
import Nyx.Ui
import Nyx.PluginView

// One widget: its plugin's view on a card (or bare), where the backend placed it. In
// edit mode: dragged to move (dropped, the backend picks the anchor and snaps it),
// buttons for size, background, workspace and removal, and a grip on parts that can be
// any size.
Item {
    id: t

    required property var tile
    required property var surface
    readonly property bool editing: surface.editing
    readonly property var item: tile.item || null
    readonly property bool bare: tile.background === "none"
    // The view shown: replaced only when the plugin sends the item again; patches go to
    // it as they come.
    property var shownView: null
    property int shownRev: -1
    property bool dragging: false
    property bool resizing: false
    property size liveSize: Qt.size(tile.w, tile.h)

    function takeView() {
        const rev = item ? item.rev : -1;
        if (rev !== shownRev) {
            shownRev = rev;
            shownView = item ? item.view : null;
        }
    }
    onItemChanged: takeView()
    Component.onCompleted: takeView()

    visible: tile.shown === true
    x: dragging ? x : tile.x
    y: dragging ? y : tile.y
    width: resizing ? liveSize.width : tile.w
    height: resizing ? liveSize.height : tile.h

    Connections {
        target: t.surface.client
        function onEvent(ev, body) {
            if (ev === "NodePatch" && t.item && body.plugin === t.tile.plugin && body.item === t.item.id && (!body.monitor || body.monitor === t.surface.connector))
                view.patch(body.nodes);
        }
    }

    Rectangle {
        id: card
        anchors.fill: parent
        radius: 15
        color: t.bare ? "transparent" : Style.card
        border.width: t.editing ? 2 : 0
        border.color: t.dragging || t.resizing ? Style.accent : Style.alpha(Style.accent, 0.55)
        clip: true

        PluginView {
            id: view
            visible: t.item !== null
            x: 16
            width: parent.width - 32
            height: Math.min(implicitHeight, parent.height - 32)
            anchors.verticalCenter: parent.verticalCenter
            view: t.shownView
            onViewEvent: (kind, control, value) => t.surface.call("ViewEvent", { id: t.tile.id, kind: kind, control: control, value: value })
        }

        // Nothing from the plugin (edit mode shows these).
        Column {
            visible: t.item === null && t.tile.stand_in !== null && t.tile.stand_in !== undefined
            x: 16
            width: parent.width - 32
            anchors.verticalCenter: parent.verticalCenter
            spacing: 4
            Label {
                width: parent.width
                text: t.tile.stand_in ? t.tile.stand_in.title : ""
                size: 15
                font.weight: Font.DemiBold
                color: Style.dimText
                wrapMode: Text.Wrap
            }
            Label {
                width: parent.width
                text: t.tile.stand_in ? t.tile.stand_in.sub : ""
                color: Style.dimText
                wrapMode: Text.Wrap
            }
        }
    }

    // Clicks reach the plugin only on interactive widgets (elsewhere the surface lets
    // them through), as commands, its popover, its menu or events.
    MouseArea {
        anchors.fill: parent
        enabled: !t.editing && t.tile.interactive === true
        acceptedButtons: Qt.LeftButton | Qt.MiddleButton | Qt.RightButton
        // Its own controls get the clicks they take; the rest is the widget's.
        propagateComposedEvents: true
        onClicked: e => {
            const button = e.button === Qt.RightButton ? 3 : e.button === Qt.MiddleButton ? 2 : 1;
            t.surface.call("Click", { id: t.tile.id, button: button }, ok => {
                if (ok && ok.menu) {
                    menu.itemId = ok.item;
                    menu.entries = ok.menu;
                    menu.openAt(e.x, e.y);
                }
            });
        }
    }
    Menu {
        id: menu
        property string itemId: ""
        anchorItem: t
        onTriggered: id => t.surface.call("MenuAction", { id: t.tile.id, action: id })
    }

    // ---------- Edit mode ----------

    MouseArea {
        id: shield
        anchors.fill: parent
        visible: t.editing
        cursorShape: t.dragging ? Qt.ClosedHandCursor : Qt.OpenHandCursor
        drag.target: t
        drag.minimumX: 0
        drag.minimumY: 0
        drag.maximumX: t.surface.width - t.width
        drag.maximumY: t.surface.height - t.height
        drag.threshold: 2
        onPressed: t.dragging = true
        onReleased: {
            if (t.dragging && (Math.round(t.x) !== t.tile.x || Math.round(t.y) !== t.tile.y))
                t.surface.call("Move", { id: t.tile.id, x: Math.round(t.x), y: Math.round(t.y) }, () => t.dragging = false);
            else
                t.dragging = false;
        }
    }

    Row {
        visible: t.editing
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: 6
        spacing: 4
        ChromeButton {
            text: (t.tile.size || "").charAt(0).toUpperCase() + (t.tile.size || "").slice(1)
            tooltip: "Size"
            onClicked: t.surface.call("CycleSize", { id: t.tile.id })
        }
        ChromeButton {
            icon: "image-x-generic-symbolic"
            on: !t.bare
            tooltip: t.bare ? "No background: click for a card" : "On a card: click for no background"
            onClicked: t.surface.call("ToggleBackground", { id: t.tile.id })
        }
        ChromeButton {
            icon: "view-pin-symbolic"
            on: !!t.tile.workspace
            tooltip: t.tile.ws_tip || ""
            onClicked: t.surface.call("ToggleWorkspace", { id: t.tile.id })
        }
        ChromeButton {
            icon: "window-close-symbolic"
            danger: true
            tooltip: "Remove"
            onClicked: t.surface.call("Remove", { id: t.tile.id })
        }
    }

    // A part that can be any size: drag its corner.
    Rectangle {
        visible: t.editing && !!t.tile.free
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        anchors.margins: 4
        width: 22
        height: 22
        color: "transparent"
        Rectangle {
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.bottom: parent.bottom
            width: 3
            color: Style.alpha(Style.accent, 0.8)
        }
        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 3
            color: Style.alpha(Style.accent, 0.8)
        }
        MouseArea {
            anchors.fill: parent
            cursorShape: Qt.SizeFDiagCursor
            property point from
            property size start
            onPressed: e => {
                from = mapToItem(t.surface.contentItem, e.x, e.y);
                start = Qt.size(t.tile.w, t.tile.h);
                t.liveSize = start;
                t.resizing = true;
            }
            onPositionChanged: e => {
                const p = mapToItem(t.surface.contentItem, e.x, e.y);
                const f = t.tile.free;
                t.liveSize = Qt.size(Math.max(f[0], Math.min(f[2], start.width + p.x - from.x)), Math.max(f[1], Math.min(f[3], start.height + p.y - from.y)));
            }
            onReleased: t.surface.call("ResizeFree", { id: t.tile.id, w: Math.round(t.liveSize.width), h: Math.round(t.liveSize.height) }, () => t.resizing = false)
        }
        Tip {
            target: parent
            text: "Drag to resize"
        }
    }
}
