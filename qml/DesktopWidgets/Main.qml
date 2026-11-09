import QtQuick
import Quickshell
import Nyx.Shell

// Nyx Desktop Widgets in the shared Quickshell runtime (NyxOS). The backend
// (nyx-desktop-widgets-backend) runs the plugins and works out where every widget sits
// and whether it can be seen; this draws one surface per monitor on the bottom layer,
// below every window.
Scope {
    id: root

    readonly property string pinnedScreen: Quickshell.env("NYX_SHELL_SCREEN") || ""
    readonly property var st: dw.state.widgets || ({ editing: false, screens: [] })

    BackendClient {
        id: dw
        app: "nyx-desktop-widgets"
        topics: ["widgets"]
    }

    Variants {
        model: root.st.screens.map(s => s.connector).filter(c => (root.pinnedScreen === "" || c === root.pinnedScreen) && Quickshell.screens.some(s => s.name === c))
        delegate: ScreenSurface {
            required property string modelData
            client: dw
            connector: modelData
            state_: root.st
        }
    }
}
