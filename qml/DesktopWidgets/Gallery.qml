import QtQuick
import Quickshell
import Nyx.Ui
import Nyx.PluginView

// **Add widget**: every widget that's turned on and not on this monitor yet, drawn live
// by its plugin (which runs while the list is open), then how to turn on the rest.
PopupWindow {
    id: g

    property Item anchorItem
    required property var surface
    property var gallery: null
    readonly property var entries: gallery ? gallery.entries || [] : []
    readonly property var off: gallery ? gallery.off || [] : []

    visible: gallery !== null && anchorItem !== null && surface.editing
    anchor.item: anchorItem
    anchor.rect.x: anchorItem ? anchorItem.width / 2 : 0
    anchor.rect.y: anchorItem ? anchorItem.height + 6 : 0
    anchor.edges: Edges.Top
    anchor.gravity: Edges.Bottom
    anchor.adjustment: PopupAdjustment.All
    grabFocus: !surface.keyboardOff
    color: "transparent"
    implicitWidth: Math.max(300, body.implicitWidth + 16)
    implicitHeight: Math.min(640, body.implicitHeight + 16)
    onVisibleChanged: if (!visible && gallery)
        surface.call("GalleryClose", {})

    Surface {
        anchors.fill: parent
        shadow: false
        Flickable {
            x: 8
            y: 8
            width: parent.width - 16
            height: parent.height - 16
            contentHeight: body.implicitHeight
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            Column {
                id: body
                spacing: 8

                Label {
                    visible: g.entries.length === 0
                    text: "Every widget that's turned on is already here."
                    color: Style.dimText
                }
                Grid {
                    columns: 2
                    columnSpacing: 12
                    rowSpacing: 12
                    Repeater {
                        model: g.entries
                        delegate: Column {
                            id: cell
                            required property var modelData
                            spacing: 6
                            Rectangle {
                                width: cell.modelData.w
                                height: cell.modelData.h
                                radius: 15
                                color: Style.card
                                clip: true
                                PluginView {
                                    visible: cell.modelData.item !== null
                                    x: 16
                                    width: parent.width - 32
                                    anchors.verticalCenter: parent.verticalCenter
                                    view: cell.modelData.item ? cell.modelData.item.view : null
                                    enabled: false
                                }
                                Label {
                                    visible: cell.modelData.item === null
                                    anchors.centerIn: parent
                                    text: cell.modelData.started ? "Nothing to show right now" : "Starting…"
                                    color: Style.dimText
                                }
                            }
                            Button {
                                text: "Add " + cell.modelData.name
                                flat: true
                                onClicked: g.surface.call("Add", { key: cell.modelData.key, size: cell.modelData.size })
                            }
                        }
                    }
                }
                Label {
                    visible: g.off.length > 0
                    topPadding: 8
                    text: "Turn on in a terminal:"
                    color: Style.dimText
                }
                Repeater {
                    model: g.off
                    delegate: Label {
                        required property var modelData
                        text: "nyx plugin configure " + modelData
                        font.family: Style.mono
                    }
                }
            }
        }
    }
}
