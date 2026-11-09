import QtQuick
import Nyx.Ui

// A small button on a widget in edit mode.
Rectangle {
    id: b

    property string text: ""
    property string icon: ""
    property string tooltip: ""
    property bool on: false
    property bool danger: false

    signal clicked

    implicitWidth: (icon !== "" ? 16 : label.implicitWidth) + 16
    implicitHeight: 24
    radius: 7
    color: area.containsMouse ? Style.mix(Style.popover, danger ? Style.danger : Style.accent, danger ? 0.45 : 0.25) : Style.alpha(Style.popover, 0.92)

    Label {
        id: label
        visible: b.icon === ""
        anchors.centerIn: parent
        text: b.text
        size: 12
    }
    Icon {
        visible: b.icon !== ""
        anchors.centerIn: parent
        name: b.icon
        size: 16
        color: b.on ? Style.accent : Style.text
    }
    MouseArea {
        id: area
        anchors.fill: parent
        hoverEnabled: true
        onClicked: b.clicked()
    }
    Tip {
        target: area
        text: b.tooltip
        shown: b.tooltip !== "" && area.containsMouse
    }
}
