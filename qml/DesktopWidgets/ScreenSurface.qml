import QtQuick
import Quickshell
import Quickshell.Wayland
import Nyx.Ui
import Nyx.PluginView

// One monitor's widgets, on the bottom layer below every window. Clicks pass through to
// the desktop except on interactive widgets; in edit mode the surface takes them all,
// for dragging widgets around, over a dimmed desktop with an 8 px grid.
PanelWindow {
    id: w

    required property var client
    required property string connector
    property var state_: ({})
    readonly property var sc: (state_.screens || []).find(s => s.connector === connector) || ({ tiles: [], area: [1, 1] })
    readonly property var tiles: sc.tiles || []
    readonly property bool editing: state_.editing === true
    readonly property bool keyboardOff: Quickshell.env("NYX_SHELL_NO_KEYBOARD") === "1" || state_.test === true
    readonly property var popover: state_.popover && state_.popover.connector === connector ? state_.popover : null
    readonly property var gallery: state_.gallery && state_.gallery.connector === connector ? state_.gallery : null

    readonly property var target: Quickshell.screens.find(s => s.name === connector) || null

    screen: target
    // A monitor showing nothing has no surface at all.
    visible: target !== null && (editing || tiles.some(t => t.shown))
    color: "transparent"
    anchors.top: true
    anchors.bottom: true
    anchors.left: true
    anchors.right: true
    exclusionMode: ExclusionMode.Normal
    exclusiveZone: 0
    WlrLayershell.layer: WlrLayer.Bottom
    WlrLayershell.namespace: "nyx-desktop-widgets"
    WlrLayershell.keyboardFocus: editing && !keyboardOff ? WlrKeyboardFocus.OnDemand : WlrKeyboardFocus.None

    // Outside edit mode only interactive widgets take clicks.
    mask: editing ? null : clickable
    Region {
        id: clickable
        regions: holes.instances
    }
    Variants {
        id: holes
        model: w.editing ? [] : w.tiles.filter(t => t.shown && t.interactive)
        delegate: Region {
            required property var modelData
            x: modelData.x
            y: modelData.y
            width: modelData.w
            height: modelData.h
        }
    }

    function call(op, body, cb) {
        client.request(op, Object.assign({ monitor: connector }, body || {}), cb || null);
    }

    FocusScope {
        id: content
        anchors.fill: parent
        focus: true
        Keys.onEscapePressed: w.call("Edit", { on: false })

        // Edit mode: the desktop dimmed, with the grid widgets snap to.
        Rectangle {
            anchors.fill: parent
            visible: w.editing
            color: Style.alpha(Style.bg, 0.45)
            Canvas {
                anchors.fill: parent
                onPaint: {
                    const ctx = getContext("2d");
                    ctx.clearRect(0, 0, width, height);
                    ctx.fillStyle = Style.alpha(Style.text, 0.05);
                    for (let x = 0; x < width; x += 8)
                        ctx.fillRect(x, 0, 1, height);
                    for (let y = 0; y < height; y += 8)
                        ctx.fillRect(0, y, width, 1);
                }
                onWidthChanged: requestPaint()
                onHeightChanged: requestPaint()
            }
        }

        Repeater {
            model: ScriptModel {
                values: w.tiles.map(t => t.id)
            }
            delegate: WidgetTile {
                required property var modelData
                tile: w.tiles.find(t => t.id === modelData) || ({ id: modelData, shown: false, x: 0, y: 0, w: 1, h: 1 })
                surface: w
            }
        }

        // Edit mode's toolbar.
        Rectangle {
            id: toolbar
            visible: w.editing
            anchors.horizontalCenter: parent.horizontalCenter
            y: 24
            width: bar.implicitWidth + 24
            height: bar.implicitHeight + 16
            radius: 15
            color: Style.popover
            Row {
                id: bar
                x: 16
                anchors.verticalCenter: parent.verticalCenter
                spacing: 8
                Label {
                    anchors.verticalCenter: parent.verticalCenter
                    text: "Drag widgets to move them"
                    color: Style.dimText
                    rightPadding: 8
                }
                Button {
                    id: addButton
                    text: "Add widget"
                    onClicked: w.call("GalleryOpen", {})
                }
                Button {
                    text: "Done"
                    primary: true
                    onClicked: w.call("Edit", { on: false })
                }
            }
        }
    }

    Gallery {
        anchorItem: addButton
        surface: w
        gallery: w.gallery
    }

    // A widget's popover (a click on an interactive widget whose plugin has one).
    PopupWindow {
        id: pop
        readonly property var tileItem: w.popover ? w.tileItem(w.popover.key) : null
        visible: w.popover !== null && tileItem !== null
        anchor.item: tileItem
        anchor.rect.x: tileItem ? tileItem.width / 2 : 0
        anchor.rect.y: tileItem ? tileItem.height + 6 : 0
        anchor.edges: Edges.Top
        anchor.gravity: Edges.Bottom
        anchor.adjustment: PopupAdjustment.All
        grabFocus: !w.keyboardOff
        color: "transparent"
        implicitWidth: Math.max(260, Math.min(480, popView.implicitWidth + 16))
        implicitHeight: Math.min(640, popView.implicitHeight + 16)
        onVisibleChanged: if (!visible && w.popover)
            w.call("PopoverClosed", {})
        Surface {
            anchors.fill: parent
            shadow: false
            PluginView {
                id: popView
                x: 8
                y: 8
                width: parent.width - 16
                view: w.popover ? w.popover.view : null
                onViewEvent: (kind, control, value) => w.call("ViewEvent", { id: w.popover.key, kind: kind, control: control, value: value })
            }
        }
        Connections {
            target: w.client
            function onEvent(ev, body) {
                if (ev === "Patch" && w.popover && body.plugin === w.popover.plugin && body.item === w.popover.item)
                    popView.patch(body.nodes);
            }
        }
    }

    // The tile on screen for a widget key (what the popover hangs from).
    function tileItem(key) {
        for (const c of content.children)
            if (c.tile && c.tile.key === key && c.visible)
                return c;
        return null;
    }
}
